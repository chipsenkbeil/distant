//! macOS FileProvider mount backend.
//!
//! Provides native Finder integration with placeholder files on macOS 12+
//! using the FileProvider framework via `objc2-file-provider`.
//!
//! **Important**: FileProvider extensions require a `.appex` inside a `.app`
//! bundle. This module provides the extension logic; the bundle assembly
//! is handled by a separate build script.
//!
//! ## Architecture
//!
//! Three Objective-C classes are defined via [`objc2::define_class!`]:
//!
//! - [`DistantFileProvider`] — implements `NSFileProviderReplicatedExtension`
//!   and `NSFileProviderEnumerating`
//! - [`DistantFileProviderItem`] — implements `NSFileProviderItemProtocol`
//! - [`DistantFileProviderEnumerator`] — implements `NSFileProviderEnumerator`
//!
//! Since the extension runs in its own `.appex` process, [`RemoteFs`] access
//! is provided via a process-global [`OnceLock`]. The container app calls
//! [`set_remote_fs`] before the extension is activated.

mod provider;
mod utils;

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use log::{debug, error, info, trace};

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::*;
use objc2_foundation::*;

use crate::core::{CacheConfig, MountConfig, RemoteFs};

use distant_core::Channel;
use distant_core::net::common::Map;
use distant_core::protocol::FileType;

/// Callback type that resolves a connection ID and destination string into
/// a [`distant_core::Channel`] by communicating with the distant manager.
pub type ChannelResolver = Box<dyn Fn(u32, &str) -> io::Result<Channel> + Send + Sync>;

/// Tokio runtime handle for the `.appex` extension process, set once by
/// [`init`] before macOS instantiates the FileProvider extension class.
static TOKIO_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Channel resolver callback, set once by [`init`] before macOS instantiates
/// the FileProvider extension class.
static CHANNEL_RESOLVER: OnceLock<ChannelResolver> = OnceLock::new();

// ---------------------------------------------------------------------------
// Global RemoteFs access
// ---------------------------------------------------------------------------

static REMOTE_FS: OnceLock<Arc<RemoteFs>> = OnceLock::new();

/// Returns a reference to the global [`RemoteFs`], if it has been set.
fn get_remote_fs() -> Option<&'static Arc<RemoteFs>> {
    REMOTE_FS.get()
}

/// Sets the global [`RemoteFs`] for the FileProvider extension process.
///
/// This must be called before any `NSFileProviderReplicatedExtension` method
/// is invoked. Subsequent calls are ignored (the first value wins).
pub(crate) fn set_remote_fs(fs: Arc<RemoteFs>) {
    let _ = REMOTE_FS.set(fs);
}

/// Registers all FileProvider ObjC classes with the Objective-C runtime.
///
/// Must be called as early as possible — before the XPC framework looks up
/// `NSExtensionPrincipalClass`. Classes defined via `define_class!` are
/// registered at runtime (not at load time like native ObjC), so the
/// framework can't find them unless this is called first.
pub(crate) fn register_classes() {
    let _: &objc2::runtime::AnyClass = <DistantFileProvider as objc2::ClassType>::class();
    let _: &objc2::runtime::AnyClass = <DistantFileProviderItem as objc2::ClassType>::class();
    let _: &objc2::runtime::AnyClass = <DistantFileProviderEnumerator as objc2::ClassType>::class();
}

/// Stores the Tokio runtime handle and channel resolver for use by the
/// `.appex` extension bootstrap flow.
///
/// Must be called once from the host process before macOS instantiates the
/// `DistantFileProvider` class via `initWithDomain:`. Subsequent calls are
/// silently ignored (the first call wins via `OnceLock`).
pub(crate) fn init(rt: tokio::runtime::Handle, resolve_channel: ChannelResolver) {
    let _ = TOKIO_HANDLE.set(rt);
    let _ = CHANNEL_RESOLVER.set(resolve_channel);
}

/// Returns the `domains/` directory inside the App Group shared container,
/// creating it if it does not exist.
///
/// Layout: `~/Library/Group Containers/39C6AGD73Z.group.dev.distant/domains/`
fn domains_dir() -> Option<PathBuf> {
    let container = crate::macos::app_group_container_path()?;
    let dir = container.join("domains");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Reads domain metadata from a file in the shared `domains/` directory and
/// initialises the global [`RemoteFs`] for this `.appex` process.
///
/// The metadata was persisted by [`register_domain`] as a serialised [`Map`]
/// in `domains/<domain_id>`.
fn bootstrap(domain_id: &str) -> io::Result<()> {
    info!("file_provider: bootstrap starting for domain {domain_id:?}");

    let dir = domains_dir().ok_or_else(|| io::Error::other("cannot resolve domains directory"))?;
    let path = dir.join(domain_id);
    info!("file_provider: reading metadata from {}", path.display());

    let value_str = std::fs::read_to_string(&path)
        .map_err(|e| io::Error::other(format!("no metadata file for domain {domain_id:?}: {e}")))?;

    let map: Map = Map::parse_json(&value_str)
        .map_err(|e| io::Error::other(format!("failed to parse domain metadata: {e}")))?;
    info!("file_provider: parsed domain metadata ({} keys)", map.len());

    // If the CLI passed a log_level, adjust the global level dynamically.
    if let Some(level_str) = map.get("log_level")
        && let Ok(level) = level_str.parse::<log::LevelFilter>()
    {
        info!("file_provider: setting log level to {level}");
        log::set_max_level(level);
    }

    let connection_id: u32 = map
        .get("connection_id")
        .ok_or_else(|| io::Error::other("domain metadata missing connection_id"))?
        .parse()
        .map_err(|e| io::Error::other(format!("invalid connection_id: {e}")))?;

    let destination = map
        .get("destination")
        .ok_or_else(|| io::Error::other("domain metadata missing destination"))?
        .clone();

    info!("file_provider: resolving channel for connection {connection_id}, dest={destination}");

    let resolver = CHANNEL_RESOLVER
        .get()
        .ok_or_else(|| io::Error::other("CHANNEL_RESOLVER not initialised — init() not called"))?;
    let channel = resolver(connection_id, &destination)?;

    let rt = TOKIO_HANDLE
        .get()
        .ok_or_else(|| io::Error::other("TOKIO_HANDLE not initialised — init() not called"))?
        .clone();

    let config = MountConfig {
        mount_point: None,
        remote_root: None,
        readonly: false,
        cache: CacheConfig::default(),
        extra: Map::new(),
    };

    let fs = RemoteFs::init(channel, config)?;
    set_remote_fs(Arc::new(fs));

    info!("file_provider: bootstrap complete — RemoteFs initialized");
    Ok(())
}

// ---------------------------------------------------------------------------
// Extracted handler functions (avoids early returns inside define_class!)
// ---------------------------------------------------------------------------

/// Handles the `itemForIdentifier:request:completionHandler:` logic.
fn handle_item_for_identifier(
    id_str: &str,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderItem, *mut NSError)>,
) {
    trace!("file_provider: handle_item_for_identifier id={id_str:?}");

    let Some(fs) = get_remote_fs() else {
        call_completion_item_error(completion_handler, "RemoteFs not initialized");
        return;
    };

    let ino: u64 = id_str.parse().unwrap_or(1);
    let attr = match fs.getattr(ino) {
        Ok(attr) => attr,
        Err(e) => {
            call_completion_item_error(completion_handler, &format!("getattr failed: {e}"));
            return;
        }
    };

    let path = fs.get_path(ino);
    let filename = path
        .as_ref()
        .map(|p| extract_filename(p.as_str()))
        .unwrap_or("unknown");

    let parent_str = if ino == 1 {
        let root_id = unsafe { NSFileProviderRootContainerItemIdentifier };
        root_id.to_string()
    } else {
        path.as_ref()
            .and_then(|p| {
                let s = p.as_str();
                let parent = s.rsplit_once('/').map(|(pp, _)| pp).unwrap_or("/");
                fs.get_ino_for_path(parent)
            })
            .map(|i| i.to_string())
            .unwrap_or_else(|| "1".to_string())
    };

    let is_dir = attr.kind == FileType::Dir;
    trace!("file_provider: item ino={ino} filename={filename:?} is_dir={is_dir}");
    let item = DistantFileProviderItem::new(id_str, &parent_str, filename, is_dir, attr.size);
    let proto: Retained<ProtocolObject<dyn NSFileProviderItemProtocol>> =
        ProtocolObject::from_retained(item);

    completion_handler.call((Retained::into_raw(proto), std::ptr::null_mut()));
}

/// Handles the `fetchContentsForItemWithIdentifier:...` logic.
fn handle_fetch_contents(
    id_str: &str,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSURL, *mut NSFileProviderItem, *mut NSError),
    >,
) {
    trace!("file_provider: handle_fetch_contents id={id_str:?}");

    let Some(fs) = get_remote_fs() else {
        call_completion_fetch_error(completion_handler, "RemoteFs not initialized");
        return;
    };

    let ino: u64 = id_str.parse().unwrap_or(0);

    let data = match fs.read(ino, 0, u32::MAX) {
        Ok(data) => {
            trace!(
                "file_provider: fetch_contents ino={ino} read {} bytes",
                data.len()
            );
            data
        }
        Err(e) => {
            call_completion_fetch_error(completion_handler, &format!("read file: {e}"));
            return;
        }
    };

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("distant_fp_{ino}"));

    if let Err(e) = std::fs::write(&tmp_path, &data) {
        call_completion_fetch_error(completion_handler, &format!("write temp file: {e}"));
        return;
    }

    // Safety: tmp_path is a valid UTF-8 path on macOS.
    let tmp_str = tmp_path.to_str().unwrap_or("");
    let url = NSURL::fileURLWithPath(&NSString::from_str(tmp_str));

    let attr = fs.getattr(ino).ok();
    let path = fs.get_path(ino);
    let filename = path
        .as_ref()
        .map(|p| extract_filename(p.as_str()))
        .unwrap_or("unknown");
    let is_dir = attr.as_ref().is_some_and(|a| a.kind == FileType::Dir);
    let size = attr.as_ref().map(|a| a.size).unwrap_or(data.len() as u64);

    let item = DistantFileProviderItem::new(id_str, "1", filename, is_dir, size);
    let proto = ProtocolObject::from_retained(item);

    completion_handler.call((
        Retained::into_raw(url),
        Retained::into_raw(proto),
        std::ptr::null_mut(),
    ));
}

/// Handles the `createItemBasedOnTemplate:...` logic.
fn handle_create_item(
    filename: &NSString,
    parent_id: &NSString,
    has_content: bool,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
    >,
) {
    let Some(fs) = get_remote_fs() else {
        call_completion_create_error(completion_handler, "RemoteFs not initialized");
        return;
    };

    let parent_ino: u64 = parent_id.to_string().parse().unwrap_or(1);
    let name = filename.to_string();

    let result = if has_content {
        fs.create(parent_ino, &name, 0o644)
    } else {
        fs.mkdir(parent_ino, &name, 0o755)
    };

    match result {
        Ok(attr) => {
            trace!(
                "file_provider: create_item succeeded — ino={} name={name:?}",
                attr.ino
            );
            let item = DistantFileProviderItem::new(
                &attr.ino.to_string(),
                &parent_ino.to_string(),
                &name,
                attr.kind == FileType::Dir,
                attr.size,
            );
            let proto = ProtocolObject::from_retained(item);
            completion_handler.call((
                Retained::into_raw(proto),
                NSFileProviderItemFields::empty(),
                Bool::NO,
                std::ptr::null_mut(),
            ));
        }
        Err(e) => {
            call_completion_create_error(completion_handler, &format!("create failed: {e}"));
        }
    }
}

/// Handles the `modifyItem:...` logic.
fn handle_modify_item(
    item_id: &NSString,
    new_contents: Option<&NSURL>,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
    >,
) {
    trace!(
        "file_provider: handle_modify_item id={:?} has_content={}",
        item_id.to_string(),
        new_contents.is_some()
    );

    let Some(fs) = get_remote_fs() else {
        call_completion_create_error(completion_handler, "RemoteFs not initialized");
        return;
    };

    let ino: u64 = item_id.to_string().parse().unwrap_or(0);

    if let Some(content_url) = new_contents
        && let Some(path_ns) = content_url.path()
    {
        let local_path = path_ns.to_string();
        match std::fs::read(&local_path) {
            Ok(data) => {
                let _ = fs.write(ino, 0, &data);
                let _ = fs.flush(ino);
            }
            Err(e) => {
                debug!("file_provider: failed to read local content: {e}");
            }
        }
    }

    let attr = fs.getattr(ino).ok();
    let path = fs.get_path(ino);
    let filename = path
        .as_ref()
        .map(|p| extract_filename(p.as_str()))
        .unwrap_or("unknown");
    let is_dir = attr.as_ref().is_some_and(|a| a.kind == FileType::Dir);
    let size = attr.as_ref().map(|a| a.size).unwrap_or(0);

    trace!("file_provider: modify_item succeeded for ino={ino}");
    let new_item = DistantFileProviderItem::new(&ino.to_string(), "1", filename, is_dir, size);
    let proto = ProtocolObject::from_retained(new_item);
    completion_handler.call((
        Retained::into_raw(proto),
        NSFileProviderItemFields::empty(),
        Bool::NO,
        std::ptr::null_mut(),
    ));
}

/// Handles the `deleteItemWithIdentifier:...` logic.
fn handle_delete_item(
    identifier: &NSFileProviderItemIdentifier,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSError)>,
) {
    trace!(
        "file_provider: handle_delete_item id={:?}",
        identifier.to_string()
    );

    let Some(fs) = get_remote_fs() else {
        let error = make_ns_error("RemoteFs not initialized");
        completion_handler.call((Retained::into_raw(error),));
        return;
    };

    let ino: u64 = identifier.to_string().parse().unwrap_or(0);

    if let Some(path) = fs.get_path(ino) {
        let path_str = path.as_str();
        if let Some((parent, name)) = path_str.rsplit_once('/') {
            let parent_path = if parent.is_empty() { "/" } else { parent };
            if let Some(parent_ino) = fs.get_ino_for_path(parent_path) {
                let result = match fs.getattr(ino) {
                    Ok(attr) if attr.kind == FileType::Dir => fs.rmdir(parent_ino, name),
                    _ => fs.unlink(parent_ino, name),
                };

                if let Err(e) = result {
                    let error = make_ns_error(&format!("delete failed: {e}"));
                    completion_handler.call((Retained::into_raw(error),));
                    return;
                }
            }
        }
    }

    trace!(
        "file_provider: delete_item succeeded for {:?}",
        identifier.to_string()
    );
    completion_handler.call((std::ptr::null_mut(),));
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Creates an `NSError` with the given message using `NSCocoaErrorDomain`.
///
/// Uses `NSCocoaErrorDomain` with code 256 (`NSFileReadUnknownError`) so that
/// macOS / Finder recognises the error domain and handles it correctly,
/// rather than hanging on an unrecognised custom domain.
fn make_ns_error(message: &str) -> Retained<NSError> {
    // SAFETY: `NSCocoaErrorDomain` is a well-known Foundation constant.
    let domain = unsafe { NSCocoaErrorDomain };
    let description = NSString::from_str(message);
    let key: &NSErrorUserInfoKey = unsafe { NSLocalizedDescriptionKey };
    let user_info = NSDictionary::from_retained_objects(
        &[key],
        &[Retained::into_super(Retained::into_super(description))],
    );
    unsafe { NSError::errorWithDomain_code_userInfo(domain, 256, Some(&user_info)) }
}

/// Creates a completed `NSProgress` for methods that execute synchronously.
fn new_progress() -> Retained<NSProgress> {
    NSProgress::discreteProgressWithTotalUnitCount(0)
}

/// Extracts the final path component from a path string.
fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Calls the item+error completion handler with an error.
fn call_completion_item_error(
    handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderItem, *mut NSError)>,
    message: &str,
) {
    let error = make_ns_error(message);
    handler.call((std::ptr::null_mut(), Retained::into_raw(error)));
}

/// Calls the fetch-contents completion handler with an error.
fn call_completion_fetch_error(
    handler: &block2::DynBlock<dyn Fn(*mut NSURL, *mut NSFileProviderItem, *mut NSError)>,
    message: &str,
) {
    let error = make_ns_error(message);
    handler.call((
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        Retained::into_raw(error),
    ));
}

/// Calls the create/modify completion handler with an error.
fn call_completion_create_error(
    handler: &block2::DynBlock<
        dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
    >,
    message: &str,
) {
    let error = make_ns_error(message);
    handler.call((
        std::ptr::null_mut(),
        NSFileProviderItemFields::empty(),
        Bool::NO,
        Retained::into_raw(error),
    ));
}

/// Queries macOS for all registered FileProvider domains, blocking until
/// the result is available.
fn get_all_domains() -> io::Result<Vec<Retained<NSFileProviderDomain>>> {
    let (tx, rx) =
        std::sync::mpsc::channel::<Result<Vec<Retained<NSFileProviderDomain>>, String>>();

    let completion = block2::RcBlock::new(
        move |domains: std::ptr::NonNull<NSArray<NSFileProviderDomain>>, error: *mut NSError| {
            let array = unsafe { domains.as_ref() };
            let vec: Vec<_> = array.iter().map(|d| d.retain()).collect();
            if !vec.is_empty() || error.is_null() {
                let _ = tx.send(Ok(vec));
            } else {
                let desc = unsafe { (*error).localizedDescription() }.to_string();
                let _ = tx.send(Err(desc));
            }
        },
    );

    unsafe {
        NSFileProviderManager::getDomainsWithCompletionHandler(&completion);
    }

    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(domains)) => Ok(domains),
        Ok(Err(e)) => Err(io::Error::other(format!(
            "getDomainsWithCompletionHandler failed: {e}"
        ))),
        Err(_) => Err(io::Error::other(
            "getDomainsWithCompletionHandler timed out",
        )),
    }
}

/// Registers a FileProvider domain with macOS.
///
/// Sets the global [`RemoteFs`] and calls `NSFileProviderManager.addDomain`
/// to register a domain. The domain identifier is derived from the
/// `connection_id` in `extra`, allowing multiple simultaneous mounts.
///
/// Domain metadata (`connection_id`, `destination`) is persisted as a file
/// in the App Group shared container (`domains/<domain_id>`) so the `.appex`
/// extension process can retrieve it later.
///
/// Before registering, any stale domains with a matching display name are
/// removed via the macOS `getDomainsWithCompletionHandler` API, ensuring
/// orphaned domains (whose metadata files were lost) are cleaned up.
///
/// # Errors
///
/// Returns an error if `connection_id` or `destination` are missing from
/// `extra`, or if the domain registration fails.
pub(crate) fn register_domain(fs: Arc<RemoteFs>, extra: &Map) -> io::Result<String> {
    set_remote_fs(fs);

    let connection_id = extra
        .get("connection_id")
        .ok_or_else(|| io::Error::other("FileProvider requires connection_id in extra map"))?;
    let destination = extra
        .get("destination")
        .ok_or_else(|| io::Error::other("FileProvider requires destination in extra map"))?;

    let domain_id = format!("dev.distant.{connection_id}");
    let display_name = sanitize_display_name(destination);

    debug!("file_provider: registering domain id={domain_id:?} display={display_name:?}");

    // Persist domain metadata as a file in the App Group shared container
    // so the .appex extension process can look up connection info by domain
    // identifier.
    let dir = domains_dir().ok_or_else(|| {
        io::Error::other(
            "cannot resolve domains directory — \
             the .appex extension will not be able to read domain metadata",
        )
    })?;
    let meta_path = dir.join(&domain_id);
    let tmp_path = dir.join(format!(".{domain_id}.tmp"));
    std::fs::write(&tmp_path, extra.to_json_string())?;
    std::fs::rename(&tmp_path, &meta_path)?;
    debug!(
        "file_provider: stored domain metadata in {}",
        meta_path.display()
    );

    // Remove any stale domain with the same display name. This uses the
    // macOS getDomainsWithCompletionHandler API to find domains even when
    // our metadata files have been lost.
    if let Ok(existing) = get_all_domains() {
        for d in &existing {
            let existing_display = unsafe { d.displayName() }.to_string();
            if existing_display == display_name {
                debug!(
                    "file_provider: removing stale domain with display name {existing_display:?}"
                );
                remove_domain_blocking(d);
            }
        }
    }

    let identifier = NSString::from_str(&domain_id);
    let display = NSString::from_str(&display_name);

    let domain = unsafe {
        NSFileProviderDomain::initWithIdentifier_displayName(
            NSFileProviderDomain::alloc(),
            &identifier,
            &display,
        )
    };

    // Also remove any domain with the exact same identifier (re-mount of the
    // same connection).
    remove_domain_blocking(&domain);

    // addDomain is async with a completion handler. We block on it using
    // a channel to bridge to sync code.
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();

    let completion = block2::RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = tx.send(None);
        } else {
            let desc = unsafe { (*error).localizedDescription() }.to_string();
            let _ = tx.send(Some(desc));
        }
    });

    unsafe {
        NSFileProviderManager::addDomain_completionHandler(&domain, &completion);
    }

    match rx.recv() {
        Ok(None) => {
            debug!("file_provider: domain {domain_id:?} registered successfully");
            Ok(domain_id)
        }
        Ok(Some(err)) => {
            debug!("file_provider: domain registration failed: {err}");
            Err(io::Error::other(format!(
                "FileProvider domain registration failed: {err}"
            )))
        }
        Err(e) => Err(io::Error::other(format!(
            "FileProvider domain registration: completion handler never called: {e}"
        ))),
    }
}

/// Sanitizes a destination string for use as a FileProvider display name.
///
/// Replaces `://` with `-` so the display name contains no slashes or colons,
/// producing clean CloudStorage folder names like `Distant-ssh-root@host`.
fn sanitize_display_name(destination: &str) -> String {
    destination.replace("://", "-")
}

/// Removes a single FileProvider domain by identifier and cleans up its
/// metadata file.
pub(crate) fn remove_domain_by_id(domain_id: &str) {
    let identifier = NSString::from_str(domain_id);
    let display = NSString::from_str("");
    let domain = unsafe {
        NSFileProviderDomain::initWithIdentifier_displayName(
            NSFileProviderDomain::alloc(),
            &identifier,
            &display,
        )
    };
    remove_domain_blocking(&domain);

    if let Some(dir) = domains_dir() {
        let _ = std::fs::remove_file(dir.join(domain_id));
    }

    debug!("file_provider: removed domain {domain_id:?}");
}

/// Removes all FileProvider domains by iterating `get_all_domains()` and
/// removing each one individually.
///
/// This avoids `removeAllDomainsWithCompletionHandler` which crashes due to
/// an ObjC binding mismatch (`removeAllDomainsForProviderIdentifier:completionHandler:`
/// receives a NULL completion handler from the objc2-file-provider binding).
pub(crate) fn remove_all_domains() -> io::Result<()> {
    let domains = get_all_domains()?;
    for domain in &domains {
        let domain_id = unsafe { domain.identifier() }.to_string();
        remove_domain_blocking(domain);
        if let Some(dir) = domains_dir() {
            let _ = std::fs::remove_file(dir.join(&domain_id));
        }
    }

    // Clean up any leftover metadata files that don't have a matching domain.
    if let Some(dir) = domains_dir()
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("dev.distant.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    Ok(())
}

/// Removes the FileProvider domain whose display name matches the sanitized
/// form of `dest`.
///
/// Uses the macOS `getDomainsWithCompletionHandler` API to enumerate all
/// registered domains, so it can find and remove orphaned domains even when
/// metadata files have been lost. Also cleans up the metadata file if present.
pub(crate) fn remove_domain_for_destination(dest: &str) -> io::Result<()> {
    let target_display = sanitize_display_name(dest);
    let domains = get_all_domains()?;

    let mut found = false;
    for domain in &domains {
        let display = unsafe { domain.displayName() }.to_string();
        if display == target_display {
            debug!("file_provider: removing domain matching destination {dest:?}");
            remove_domain_blocking(domain);

            // Clean up metadata file if it exists.
            let domain_id = unsafe { domain.identifier() }.to_string();
            if let Some(dir) = domains_dir() {
                let _ = std::fs::remove_file(dir.join(&domain_id));
            }

            found = true;
        }
    }

    if found {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "no FileProvider domain found for destination {dest}"
        )))
    }
}

/// Removes a FileProvider domain, blocking until the operation completes.
/// Errors are logged but not propagated — removal is best-effort cleanup.
fn remove_domain_blocking(domain: &NSFileProviderDomain) {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();

    let completion = block2::RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = tx.send(None);
        } else {
            let desc = unsafe { (*error).localizedDescription() }.to_string();
            let _ = tx.send(Some(desc));
        }
    });

    unsafe {
        NSFileProviderManager::removeDomain_completionHandler(domain, &completion);
    }

    match rx.recv() {
        Ok(None) => debug!("file_provider: removed existing domain"),
        Ok(Some(err)) => debug!("file_provider: remove domain (best-effort): {err}"),
        Err(_) => {}
    }
}

/// Stores the Tokio runtime handle and channel resolver needed by the
/// `.appex` FileProvider extension bootstrap flow.
///
/// Subsequent calls are silently ignored (the first call wins).
pub fn init_file_provider(rt: tokio::runtime::Handle, resolve_channel: ChannelResolver) {
    crate::backend::macos_file_provider::init(rt, resolve_channel);
}

/// Registers FileProvider ObjC classes with the Objective-C runtime.
///
/// Must be called before the XPC framework looks up
/// `NSExtensionPrincipalClass`, as classes defined via `define_class!`
/// are registered at runtime rather than at load time.
pub fn register_file_provider_classes() {
    crate::backend::macos_file_provider::register_classes();
}

/// Removes all distant FileProvider domains and cleans up their metadata.
///
/// # Errors
///
/// Returns an error if domain enumeration fails.
pub fn remove_all_file_provider_domains() -> io::Result<()> {
    crate::backend::macos_file_provider::remove_all_domains()
}

/// Removes the FileProvider domain whose stored destination matches `dest`.
///
/// Used during unmount-by-destination: e.g. `distant unmount ssh://root@host`.
///
/// # Errors
///
/// Returns an error if no matching domain is found.
pub fn remove_file_provider_domain_for_destination(dest: &str) -> io::Result<()> {
    crate::backend::macos_file_provider::remove_domain_for_destination(dest)
}
