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
//! Since the extension runs in its own `.appex` process, [`Runtime`] access
//! is provided via a process-global [`OnceLock`]. The container app calls
//! [`init`] before the extension is activated.

mod provider;
pub(crate) mod utils;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use log::{debug, info, trace};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{AnyThread, Message};
use objc2_file_provider::*;
use objc2_foundation::*;

use crate::core::{CacheConfig, MountConfig, Runtime};

use distant_core::Channel;
use distant_core::net::common::Map;
use distant_core::protocol::FileType;

use provider::{DistantFileProvider, DistantFileProviderEnumerator, DistantFileProviderItem};

/// Callback type that resolves a connection ID and destination string into
/// a [`distant_core::Channel`] by communicating with the distant manager.
pub type ChannelResolver = Box<dyn Fn(u32, &str) -> io::Result<Channel> + Send + Sync>;

/// Wrapper for Apple-provided ObjC objects documented as thread-safe
/// but `!Send` due to conservative defaults in block2/objc2.
///
/// Implements [`Deref`](std::ops::Deref) so callers can invoke methods on
/// the inner type without accessing the field directly. Direct field access
/// (`wrapper.0`) causes async `Send`-checking to see the inner `!Send` type,
/// while method calls through `Deref` only see this wrapper (which is `Send`).
pub(crate) struct UnsafeSendable<T>(T);

// SAFETY: The wrapped types (RcBlock completion handlers, Retained protocol
// objects) are used to call ObjC methods that are documented as thread-safe.
// The `!Send`/`!Sync` bounds come from the generic ObjC runtime wrappers,
// not from actual thread-safety concerns with these specific types.
unsafe impl<T> Send for UnsafeSendable<T> {}
unsafe impl<T> Sync for UnsafeSendable<T> {}

impl<T> std::ops::Deref for UnsafeSendable<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Tokio runtime handle for the `.appex` extension process, set once by
/// [`init`] before macOS instantiates the FileProvider extension class.
static TOKIO_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Channel resolver callback, set once by [`init`] before macOS instantiates
/// the FileProvider extension class.
static CHANNEL_RESOLVER: OnceLock<ChannelResolver> = OnceLock::new();

// ---------------------------------------------------------------------------
// Global Runtime access
// ---------------------------------------------------------------------------

/// Per-domain Runtimes, keyed by domain identifier.
///
/// Supports multiple simultaneous mounts in the same `.appex` process.
/// The host app path (`register_domain`) and the appex path (`bootstrap`)
/// both insert into this map.
static RUNTIMES: RwLock<Option<HashMap<String, Arc<Runtime>>>> = RwLock::new(None);

/// Per-domain bootstrap error messages.
static BOOTSTRAP_ERRORS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Returns the [`Runtime`] for the given domain, if it has been set.
pub(crate) fn get_runtime(domain_id: &str) -> Option<Arc<Runtime>> {
    RUNTIMES.read().ok()?.as_ref()?.get(domain_id).cloned()
}

/// Returns the bootstrap error message for a domain, if bootstrap failed.
pub(crate) fn get_bootstrap_error(domain_id: &str) -> Option<String> {
    BOOTSTRAP_ERRORS
        .read()
        .ok()?
        .as_ref()?
        .get(domain_id)
        .cloned()
}

/// Stores a bootstrap error message for the given domain.
pub(crate) fn store_bootstrap_error(domain_id: &str, message: String) {
    if let Ok(mut guard) = BOOTSTRAP_ERRORS.write() {
        guard
            .get_or_insert_with(HashMap::new)
            .insert(domain_id.to_owned(), message);
    }
}

/// Stores a [`Runtime`] for the given domain.
fn set_runtime(domain_id: &str, rt: Arc<Runtime>) {
    if let Ok(mut guard) = RUNTIMES.write() {
        guard
            .get_or_insert_with(HashMap::new)
            .insert(domain_id.to_owned(), rt);
    }
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
    let container = utils::app_group_container_path()?;
    let dir = container.join("domains");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Reads domain metadata from a file in the shared `domains/` directory and
/// initialises the global [`Runtime`] for this `.appex` process.
///
/// The metadata was persisted by [`register_domain`] as a serialised [`Map`]
/// in `domains/<domain_id>`.
pub(crate) fn bootstrap(domain_id: &str) -> io::Result<()> {
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

    let handle = TOKIO_HANDLE
        .get()
        .ok_or_else(|| io::Error::other("TOKIO_HANDLE not initialised — init() not called"))?
        .clone();

    let resolver = CHANNEL_RESOLVER
        .get()
        .ok_or_else(|| io::Error::other("CHANNEL_RESOLVER not initialised — init() not called"))?;

    // Resolve the channel synchronously here so we can fail fast with
    // a clear error if the manager is unreachable.
    let channel = resolver(connection_id, &destination)?;

    let remote_root = map
        .get("remote_root")
        .map(distant_core::protocol::RemotePath::new);

    let readonly = map.get("readonly").is_some_and(|v| v == "true");

    let config = MountConfig {
        mount_point: None,
        remote_root,
        readonly,
        cache: CacheConfig::default(),
        extra: Map::new(),
    };

    // Create Runtime with async init — the RemoteFs initialization happens
    // in the background. All handler spawn() calls will wait for init.
    let rt = Arc::new(Runtime::new(handle, async move { (channel, config) }));

    set_runtime(domain_id, rt.clone());

    // Warm the cache by pre-enumerating the root directory in the
    // background. This ensures the first Finder open is instant.
    rt.spawn(|fs| async move {
        match fs.readdir(1).await {
            Ok(entries) => {
                info!(
                    "file_provider: cache warm complete — root has {} entries",
                    entries.len()
                );
            }
            Err(e) => {
                debug!("file_provider: cache warm failed (non-fatal): {e}");
            }
        }
    });

    info!("file_provider: bootstrap complete — Runtime initialized (init pending)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Extracted handler functions (avoids early returns inside define_class!)
// ---------------------------------------------------------------------------

/// Handles the `itemForIdentifier:request:completionHandler:` logic.
pub(crate) fn handle_item_for_identifier(
    domain_id: &str,
    id_str: &str,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderItem, *mut NSError)>,
) {
    trace!("file_provider: handle_item_for_identifier domain={domain_id:?} id={id_str:?}");

    // Working set and trash containers are not backed by real items.
    let working_set_id = unsafe { NSFileProviderWorkingSetContainerItemIdentifier }.to_string();
    let trash_id = unsafe { NSFileProviderTrashContainerItemIdentifier }.to_string();
    if id_str == working_set_id || id_str == trash_id {
        let error = make_fp_error(
            NSFileProviderErrorCode::NoSuchItem,
            &format!("container {id_str:?} has no backing item"),
        );
        completion_handler.call((std::ptr::null_mut(), Retained::into_raw(error)));
        return;
    }

    let Some(rt) = get_runtime(domain_id) else {
        call_completion_item_error(completion_handler, "Runtime not initialized");
        return;
    };

    // Detect the root container constant and map it to inode 1.
    let root_id_str = unsafe { NSFileProviderRootContainerItemIdentifier }.to_string();
    let is_root = id_str == root_id_str;
    let ino: u64 = if is_root {
        1
    } else {
        match id_str.parse() {
            Ok(n) => n,
            Err(_) => {
                let error = make_fp_error(
                    NSFileProviderErrorCode::NoSuchItem,
                    &format!("unknown identifier {id_str:?}"),
                );
                completion_handler.call((std::ptr::null_mut(), Retained::into_raw(error)));
                return;
            }
        }
    };

    let block = UnsafeSendable(completion_handler.copy());

    rt.spawn(move |fs| async move {
        match fs.getattr(ino).await {
            Ok(attr) => {
                // For the root item, use the framework constants for both
                // the item identifier and parent identifier.
                let (item_id_str, parent_str, filename) = if is_root {
                    (root_id_str.clone(), root_id_str, "/".to_owned())
                } else {
                    let path = fs.get_path(ino).await;
                    let fname = path
                        .as_ref()
                        .map(|p| extract_filename(p.as_str()).to_owned())
                        .unwrap_or_else(|| "unknown".to_owned());

                    let parent = if let Some(ref p) = path {
                        let s = p.as_str();
                        let parent_path = s.rsplit_once('/').map(|(pp, _)| pp).unwrap_or("/");
                        let parent_ino = fs.get_ino_for_path(parent_path).await;
                        match parent_ino {
                            Some(1) => root_id_str.clone(),
                            Some(i) => i.to_string(),
                            None => root_id_str.clone(),
                        }
                    } else {
                        root_id_str.clone()
                    };

                    (ino.to_string(), parent, fname)
                };

                let is_dir = attr.kind == FileType::Dir;
                let mtime_secs = attr
                    .mtime
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                trace!("file_provider: item ino={ino} filename={filename:?} is_dir={is_dir}");
                let item = DistantFileProviderItem::new(
                    &item_id_str,
                    &parent_str,
                    &filename,
                    is_dir,
                    attr.size,
                    mtime_secs,
                );
                let proto: Retained<
                    objc2::runtime::ProtocolObject<dyn NSFileProviderItemProtocol>,
                > = objc2::runtime::ProtocolObject::from_retained(item);

                block.call((Retained::into_raw(proto), std::ptr::null_mut()));
            }
            Err(e) => {
                let error = make_ns_error(&format!("getattr failed: {e}"));
                block.call((std::ptr::null_mut(), Retained::into_raw(error)));
            }
        }
    });
}

/// Handles the `fetchContentsForItemWithIdentifier:...` logic.
pub(crate) fn handle_fetch_contents(
    domain_id: &str,
    id_str: &str,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSURL, *mut NSFileProviderItem, *mut NSError),
    >,
) {
    trace!("file_provider: handle_fetch_contents id={id_str:?}");

    let Some(rt) = get_runtime(domain_id) else {
        call_completion_fetch_error(completion_handler, "Runtime not initialized");
        return;
    };

    let block = UnsafeSendable(completion_handler.copy());
    let ino: u64 = id_str.parse().unwrap_or(0);

    rt.spawn(move |fs| async move {
        let data = match fs.read(ino, 0, u32::MAX).await {
            Ok(data) => {
                trace!(
                    "file_provider: fetch_contents ino={ino} read {} bytes",
                    data.len()
                );
                data
            }
            Err(e) => {
                let error = make_ns_error(&format!("read file: {e}"));
                block.call((
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    Retained::into_raw(error),
                ));
                return;
            }
        };

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("distant_fp_{ino}"));

        if let Err(e) = std::fs::write(&tmp_path, &data) {
            let error = make_ns_error(&format!("write temp file: {e}"));
            block.call((
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                Retained::into_raw(error),
            ));
            return;
        }

        let tmp_str = tmp_path.to_str().unwrap_or("");
        let url = NSURL::fileURLWithPath(&NSString::from_str(tmp_str));

        let attr = fs.getattr(ino).await.ok();
        let path = fs.get_path(ino).await;
        let filename = path
            .as_ref()
            .map(|p| extract_filename(p.as_str()))
            .unwrap_or("unknown");
        let is_dir = attr.as_ref().is_some_and(|a| a.kind == FileType::Dir);
        let size = attr.as_ref().map(|a| a.size).unwrap_or(data.len() as u64);
        let mtime_secs = attr
            .as_ref()
            .map(|a| {
                a.mtime
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let parent_str =
            resolve_parent_identifier(&fs, ino, path.as_ref().map(|p| p.as_str())).await;
        let item = DistantFileProviderItem::new(
            &ino.to_string(),
            &parent_str,
            filename,
            is_dir,
            size,
            mtime_secs,
        );
        let proto = objc2::runtime::ProtocolObject::from_retained(item);

        block.call((
            Retained::into_raw(url),
            Retained::into_raw(proto),
            std::ptr::null_mut(),
        ));
    });
}

/// Handles the `createItemBasedOnTemplate:...` logic.
pub(crate) fn handle_create_item(
    domain_id: &str,
    filename: &NSString,
    parent_id: &NSString,
    is_dir: bool,
    content_data: Option<Vec<u8>>,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
    >,
) {
    let Some(rt) = get_runtime(domain_id) else {
        call_completion_create_error(completion_handler, "Runtime not initialized");
        return;
    };

    let block = UnsafeSendable(completion_handler.copy());
    let parent_id_string = parent_id.to_string();
    let root_id_str = unsafe { NSFileProviderRootContainerItemIdentifier }.to_string();
    let parent_ino: u64 = if parent_id_string == root_id_str {
        1
    } else {
        parent_id_string.parse().unwrap_or(1)
    };
    let name = filename.to_string();

    rt.spawn(move |fs| async move {
        let result = if is_dir {
            fs.mkdir(parent_ino, &name, 0o755).await
        } else {
            fs.create(parent_ino, &name, 0o644).await
        };

        match result {
            Ok(attr) => {
                // Write file content if provided (e.g., drag-and-drop into Finder).
                if let Some(data) = content_data {
                    trace!(
                        "file_provider: writing {} bytes to new file ino={}",
                        data.len(),
                        attr.ino,
                    );
                    if let Err(e) = fs.write(attr.ino, 0, &data).await {
                        let error = make_ns_error(&format!("write content failed: {e}"));
                        block.call((
                            std::ptr::null_mut(),
                            NSFileProviderItemFields::empty(),
                            Bool::NO,
                            Retained::into_raw(error),
                        ));
                        return;
                    }
                    let _ = fs.flush(attr.ino).await;
                }

                trace!(
                    "file_provider: create_item succeeded — ino={} name={name:?}",
                    attr.ino
                );
                // Re-fetch attr after writing content so size/mtime are current.
                let attr = fs.getattr(attr.ino).await.unwrap_or(attr);
                let mtime_secs = attr
                    .mtime
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let parent_id_str = if parent_ino == 1 {
                    unsafe { NSFileProviderRootContainerItemIdentifier }.to_string()
                } else {
                    parent_ino.to_string()
                };
                let item = DistantFileProviderItem::new(
                    &attr.ino.to_string(),
                    &parent_id_str,
                    &name,
                    attr.kind == FileType::Dir,
                    attr.size,
                    mtime_secs,
                );
                let proto = objc2::runtime::ProtocolObject::from_retained(item);
                block.call((
                    Retained::into_raw(proto),
                    NSFileProviderItemFields::empty(),
                    Bool::NO,
                    std::ptr::null_mut(),
                ));
            }
            Err(e) => {
                let error = make_ns_error(&format!("create failed: {e}"));
                block.call((
                    std::ptr::null_mut(),
                    NSFileProviderItemFields::empty(),
                    Bool::NO,
                    Retained::into_raw(error),
                ));
            }
        }
    });
}

/// Handles the `modifyItem:...` logic.
///
/// Supports content writes (when `local_data` is `Some`), renames (when
/// `new_filename` is `Some`), moves (when `new_parent_id` is `Some`), and
/// combined rename-and-move operations.
pub(crate) fn handle_modify_item(
    domain_id: &str,
    item_id: &NSString,
    new_filename: Option<String>,
    new_parent_id: Option<String>,
    new_contents: Option<&NSURL>,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
    >,
) {
    trace!(
        "file_provider: handle_modify_item id={:?} has_content={} rename={} move={}",
        item_id.to_string(),
        new_contents.is_some(),
        new_filename.is_some(),
        new_parent_id.is_some(),
    );

    let Some(rt) = get_runtime(domain_id) else {
        call_completion_create_error(completion_handler, "Runtime not initialized");
        return;
    };

    let block = UnsafeSendable(completion_handler.copy());
    let ino: u64 = item_id.to_string().parse().unwrap_or(0);

    // Read local file content before spawning (NSURL is not Send).
    let local_data = new_contents.and_then(|content_url| {
        content_url
            .path()
            .map(|path_ns| path_ns.to_string())
            .and_then(|local_path| std::fs::read(&local_path).ok())
    });

    let root_id_str = unsafe { NSFileProviderRootContainerItemIdentifier }.to_string();

    rt.spawn(move |fs| async move {
        // Write new content if provided.
        if let Some(data) = local_data {
            if let Err(e) = fs.write(ino, 0, &data).await {
                let error = make_ns_error(&format!("write content failed: {e}"));
                block.call((
                    std::ptr::null_mut(),
                    NSFileProviderItemFields::empty(),
                    Bool::NO,
                    Retained::into_raw(error),
                ));
                return;
            }
            let _ = fs.flush(ino).await;
        }

        // Perform rename/move if filename or parent changed.
        if new_filename.is_some() || new_parent_id.is_some() {
            let rename_result: Result<(), String> = async {
                let current_path = fs
                    .get_path(ino)
                    .await
                    .ok_or_else(|| format!("unknown inode {ino}"))?;
                let current_str = current_path.as_str();
                let (current_parent, current_name) = current_str
                    .rsplit_once('/')
                    .ok_or_else(|| format!("invalid path: {current_str}"))?;
                let current_parent = if current_parent.is_empty() {
                    "/"
                } else {
                    current_parent
                };
                let old_parent_ino = fs
                    .get_ino_for_path(current_parent)
                    .await
                    .ok_or_else(|| format!("unknown parent path: {current_parent}"))?;

                // Resolve the new parent inode: use the provided parent ID if
                // the parent changed, otherwise keep the current parent.
                let new_parent_ino = if let Some(ref pid) = new_parent_id {
                    if *pid == root_id_str {
                        1
                    } else {
                        pid.parse::<u64>()
                            .map_err(|_| format!("invalid parent id: {pid}"))?
                    }
                } else {
                    old_parent_ino
                };

                let rename_name = new_filename.as_deref().unwrap_or(current_name);

                fs.rename(old_parent_ino, current_name, new_parent_ino, rename_name)
                    .await
                    .map_err(|e| format!("rename failed: {e}"))
            }
            .await;

            if let Err(e) = rename_result {
                debug!("file_provider: modify_item rename failed: {e}");
                let error = make_ns_error(&e);
                block.call((
                    std::ptr::null_mut(),
                    NSFileProviderItemFields::empty(),
                    Bool::NO,
                    Retained::into_raw(error),
                ));
                return;
            }
        }

        // Re-fetch attributes and path after all mutations so the returned
        // item reflects the final state.
        let attr = fs.getattr(ino).await.ok();
        let path = fs.get_path(ino).await;
        let filename = path
            .as_ref()
            .map(|p| extract_filename(p.as_str()))
            .unwrap_or("unknown");
        let is_dir = attr.as_ref().is_some_and(|a| a.kind == FileType::Dir);
        let size = attr.as_ref().map(|a| a.size).unwrap_or(0);

        let mtime_secs = attr
            .as_ref()
            .map(|a| {
                a.mtime
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let parent_str =
            resolve_parent_identifier(&fs, ino, path.as_ref().map(|p| p.as_str())).await;
        trace!("file_provider: modify_item succeeded for ino={ino}");
        let new_item = DistantFileProviderItem::new(
            &ino.to_string(),
            &parent_str,
            filename,
            is_dir,
            size,
            mtime_secs,
        );
        let proto = objc2::runtime::ProtocolObject::from_retained(new_item);
        block.call((
            Retained::into_raw(proto),
            NSFileProviderItemFields::empty(),
            Bool::NO,
            std::ptr::null_mut(),
        ));
    });
}

/// Handles the `deleteItemWithIdentifier:...` logic.
pub(crate) fn handle_delete_item(
    domain_id: &str,
    identifier: &NSFileProviderItemIdentifier,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSError)>,
) {
    trace!(
        "file_provider: handle_delete_item id={:?}",
        identifier.to_string()
    );

    let Some(rt) = get_runtime(domain_id) else {
        let error = make_ns_error("Runtime not initialized");
        completion_handler.call((Retained::into_raw(error),));
        return;
    };

    let block = UnsafeSendable(completion_handler.copy());
    let ino: u64 = identifier.to_string().parse().unwrap_or(0);

    rt.spawn(move |fs| async move {
        let result: Result<(), String> = async {
            let path = fs
                .get_path(ino)
                .await
                .ok_or_else(|| format!("unknown inode {ino}"))?;
            let path_str = path.as_str();
            let (parent, name) = path_str
                .rsplit_once('/')
                .ok_or_else(|| format!("invalid path: {path_str}"))?;
            let parent_path = if parent.is_empty() { "/" } else { parent };
            let parent_ino = fs
                .get_ino_for_path(parent_path)
                .await
                .ok_or_else(|| format!("unknown parent path: {parent_path}"))?;

            let delete_result = match fs.getattr(ino).await {
                Ok(attr) if attr.kind == FileType::Dir => fs.rmdir(parent_ino, name).await,
                _ => fs.unlink(parent_ino, name).await,
            };
            delete_result.map_err(|e| format!("delete failed: {e}"))
        }
        .await;

        match result {
            Ok(()) => {
                trace!("file_provider: delete_item succeeded for ino={ino}");
                block.call((std::ptr::null_mut(),));
            }
            Err(e) => {
                debug!("file_provider: delete_item failed: {e}");
                let error = make_ns_error(&e);
                block.call((Retained::into_raw(error),));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Creates an `NSError` with the given message using `NSCocoaErrorDomain`.
///
/// Uses `NSCocoaErrorDomain` with code 256 (`NSFileReadUnknownError`) so that
/// macOS / Finder recognises the error domain and handles it correctly,
/// rather than hanging on an unrecognised custom domain.
pub(crate) fn make_ns_error(message: &str) -> Retained<NSError> {
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

/// Creates an `NSError` using `NSFileProviderErrorDomain` with the specified
/// error code.
///
/// Use this for errors that Finder should display with FileProvider-specific
/// UI (e.g., "server unreachable" offline indicator).
pub(crate) fn make_fp_error(code: NSFileProviderErrorCode, message: &str) -> Retained<NSError> {
    let domain = unsafe { NSFileProviderErrorDomain };
    let description = NSString::from_str(message);
    let key: &NSErrorUserInfoKey = unsafe { NSLocalizedDescriptionKey };
    let user_info = NSDictionary::from_retained_objects(
        &[key],
        &[Retained::into_super(Retained::into_super(description))],
    );
    unsafe { NSError::errorWithDomain_code_userInfo(domain, code.0, Some(&user_info)) }
}

/// Creates a completed `NSProgress` for methods that execute synchronously.
pub(crate) fn new_progress() -> Retained<NSProgress> {
    NSProgress::discreteProgressWithTotalUnitCount(0)
}

/// Extracts the final path component from a path string.
fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Removes temporary files created by `fetchContents` (`/tmp/distant_fp_*`).
fn cleanup_temp_files() {
    let tmp_dir = std::env::temp_dir();
    if let Ok(entries) = std::fs::read_dir(&tmp_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("distant_fp_") {
                debug!("file_provider: cleaning up temp file {}", name);
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Resolves the parent item identifier for a given inode.
///
/// Returns `NSFileProviderRootContainerItemIdentifier` when the parent is
/// the root (inode 1), otherwise returns the numeric inode string.
async fn resolve_parent_identifier(
    fs: &crate::core::RemoteFs,
    ino: u64,
    path: Option<&str>,
) -> String {
    let root_id_str = unsafe { NSFileProviderRootContainerItemIdentifier }.to_string();

    if ino == 1 {
        return root_id_str;
    }

    if let Some(s) = path {
        let parent_path = s.rsplit_once('/').map(|(pp, _)| pp).unwrap_or("/");
        let parent_path = if parent_path.is_empty() {
            "/"
        } else {
            parent_path
        };
        match fs.get_ino_for_path(parent_path).await {
            Some(1) => root_id_str,
            Some(i) => i.to_string(),
            None => root_id_str,
        }
    } else {
        root_id_str
    }
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
/// Stores the provided [`Runtime`] in the per-domain map and calls
/// `NSFileProviderManager.addDomain` to register a domain. The domain
/// identifier is derived from `connection_id` and `remote_root` in `extra`,
/// allowing multiple simultaneous mounts to different paths.
///
/// Domain metadata is persisted as a file in the App Group shared container
/// (`domains/<domain_id>`) so the `.appex` extension process can bootstrap.
///
/// If a domain with the same identifier already exists (re-mount of the
/// same connection+root), it is removed before re-adding.
///
/// # Errors
///
/// Returns an error if `connection_id` or `destination` are missing from
/// `extra`, or if the domain registration fails.
pub(crate) fn register_domain(rt: Arc<Runtime>, extra: &Map) -> io::Result<String> {
    let connection_id = extra
        .get("connection_id")
        .ok_or_else(|| io::Error::other("FileProvider requires connection_id in extra map"))?;
    let destination = extra
        .get("destination")
        .ok_or_else(|| io::Error::other("FileProvider requires destination in extra map"))?;

    let remote_root = extra.get("remote_root");
    let domain_id = make_domain_id(connection_id, remote_root);
    set_runtime(&domain_id, rt);
    let display_name = sanitize_display_name(destination, remote_root);

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

/// Builds a domain identifier from connection ID and optional remote root.
///
/// Without a remote root: `dev.distant.{connection_id}`
/// With a remote root: `dev.distant.{connection_id}.{hash}` where hash is
/// a truncated hash of the remote root path.
fn make_domain_id(connection_id: &str, remote_root: Option<&String>) -> String {
    match remote_root {
        Some(root) => {
            // Simple hash: sum of bytes mod a large prime, hex-encoded.
            let hash: u64 = root.bytes().fold(0u64, |acc, b| {
                acc.wrapping_mul(31).wrapping_add(u64::from(b))
            });
            format!("dev.distant.{connection_id}.{hash:x}")
        }
        None => format!("dev.distant.{connection_id}"),
    }
}

/// Builds the FileProvider domain display name from destination and remote root.
///
/// macOS prepends the extension's `CFBundleDisplayName` ("Distant") automatically,
/// so the display name is just the connection-specific part. Examples:
/// - `ssh://root@host`
/// - `ssh://root@host:/var/data`
fn sanitize_display_name(destination: &str, remote_root: Option<&String>) -> String {
    match remote_root {
        Some(root) => format!("{destination}:{root}"),
        None => destination.to_owned(),
    }
}

/// Removes a single FileProvider domain by identifier and cleans up its
/// metadata file.
#[allow(dead_code)]
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

    cleanup_temp_files();
    Ok(())
}

/// Removes all FileProvider domains whose stored destination matches `dest`.
///
/// Matches by reading domain metadata files and comparing the `destination`
/// field, rather than relying on display name format. This correctly handles
/// domains with different remote roots on the same destination.
pub(crate) fn remove_domain_for_destination(dest: &str) -> io::Result<()> {
    let domains = get_all_domains()?;

    let mut found = false;
    for domain in &domains {
        let domain_id = unsafe { domain.identifier() }.to_string();

        // Check if the domain's metadata matches the target destination.
        let matches = domains_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join(&domain_id)).ok())
            .and_then(|s| Map::parse_json(&s).ok())
            .is_some_and(|meta| meta.get("destination").is_some_and(|d| d == dest));

        if matches {
            debug!("file_provider: removing domain {domain_id:?} matching destination {dest:?}");
            remove_domain_blocking(domain);

            if let Some(dir) = domains_dir() {
                let _ = std::fs::remove_file(dir.join(&domain_id));
            }

            found = true;
        }
    }

    if found {
        cleanup_temp_files();
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

// ---------------------------------------------------------------------------
// Public API wrappers (used by the binary crate)
// ---------------------------------------------------------------------------

/// Stores the Tokio runtime handle and channel resolver needed by the
/// `.appex` FileProvider extension bootstrap flow.
///
/// Subsequent calls are silently ignored (the first call wins).
pub fn init_file_provider(rt: tokio::runtime::Handle, resolve_channel: ChannelResolver) {
    init(rt, resolve_channel);
}

/// Registers FileProvider ObjC classes with the Objective-C runtime.
///
/// Must be called before the XPC framework looks up
/// `NSExtensionPrincipalClass`, as classes defined via `define_class!`
/// are registered at runtime rather than at load time.
pub fn register_file_provider_classes() {
    register_classes();
}

/// Removes all distant FileProvider domains and cleans up their metadata.
///
/// # Errors
///
/// Returns an error if domain enumeration fails.
pub fn remove_all_file_provider_domains() -> io::Result<()> {
    remove_all_domains()
}

/// Removes the FileProvider domain whose stored destination matches `dest`.
///
/// Used during unmount-by-destination: e.g. `distant unmount ssh://root@host`.
///
/// # Errors
///
/// Returns an error if no matching domain is found.
pub fn remove_file_provider_domain_for_destination(dest: &str) -> io::Result<()> {
    remove_domain_for_destination(dest)
}

/// Information about a registered FileProvider domain.
#[derive(Debug)]
pub struct DomainInfo {
    /// Domain identifier (e.g., `dev.distant.42`).
    pub identifier: String,
    /// Display name shown in Finder sidebar.
    pub display_name: String,
    /// Connection ID from domain metadata, if available.
    pub connection_id: Option<u32>,
    /// Destination string from domain metadata, if available.
    pub destination: Option<String>,
    /// Whether the domain metadata file exists in the App Group container.
    pub has_metadata: bool,
}

/// Lists all registered FileProvider domains with their metadata.
///
/// # Errors
///
/// Returns an error if domain enumeration fails.
pub fn list_file_provider_domains() -> io::Result<Vec<DomainInfo>> {
    let domains = get_all_domains()?;
    let mut result = Vec::with_capacity(domains.len());

    for domain in &domains {
        let identifier = unsafe { domain.identifier() }.to_string();
        let display_name = unsafe { domain.displayName() }.to_string();

        let (connection_id, destination, has_metadata) = if let Some(dir) = domains_dir() {
            let meta_path = dir.join(&identifier);
            if meta_path.exists() {
                let meta = std::fs::read_to_string(&meta_path)
                    .ok()
                    .and_then(|s| distant_core::net::common::Map::parse_json(&s).ok());
                let conn_id = meta
                    .as_ref()
                    .and_then(|m| m.get("connection_id"))
                    .and_then(|s| s.parse().ok());
                let dest = meta.as_ref().and_then(|m| m.get("destination")).cloned();
                (conn_id, dest, true)
            } else {
                (None, None, false)
            }
        } else {
            (None, None, false)
        };

        result.push(DomainInfo {
            identifier,
            display_name,
            connection_id,
            destination,
            has_metadata,
        });
    }

    Ok(result)
}
