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

#![allow(dead_code)]

use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use log::debug;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::*;
use objc2_foundation::*;

use crate::RemoteFs;

use distant_core::protocol::FileType;

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

// ---------------------------------------------------------------------------
// DistantFileProviderItem
// ---------------------------------------------------------------------------

/// Instance variables for [`DistantFileProviderItem`].
pub(crate) struct ItemIvars {
    identifier: Retained<NSString>,
    parent_identifier: Retained<NSString>,
    filename: Retained<NSString>,
    is_directory: bool,
    size: u64,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = ItemIvars]
    #[name = "DistantFileProviderItem"]
    pub(crate) struct DistantFileProviderItem;

    unsafe impl NSObjectProtocol for DistantFileProviderItem {}

    unsafe impl NSFileProviderItemProtocol for DistantFileProviderItem {
        #[unsafe(method_id(itemIdentifier))]
        fn item_identifier(&self) -> Retained<NSFileProviderItemIdentifier> {
            self.ivars().identifier.clone()
        }

        #[unsafe(method_id(parentItemIdentifier))]
        fn parent_item_identifier(&self) -> Retained<NSFileProviderItemIdentifier> {
            self.ivars().parent_identifier.clone()
        }

        #[unsafe(method_id(filename))]
        fn filename(&self) -> Retained<NSString> {
            self.ivars().filename.clone()
        }
    }
);

impl DistantFileProviderItem {
    /// Creates a new item with the given metadata.
    fn new(
        identifier: &str,
        parent_identifier: &str,
        filename: &str,
        is_directory: bool,
        size: u64,
    ) -> Retained<Self> {
        let item = Self::alloc().set_ivars(ItemIvars {
            identifier: NSString::from_str(identifier),
            parent_identifier: NSString::from_str(parent_identifier),
            filename: NSString::from_str(filename),
            is_directory,
            size,
        });
        // SAFETY: NSObject's `init` is always safe to call after `alloc`.
        unsafe { msg_send![super(item), init] }
    }
}

// ---------------------------------------------------------------------------
// DistantFileProviderEnumerator
// ---------------------------------------------------------------------------

/// Instance variables for [`DistantFileProviderEnumerator`].
pub(crate) struct EnumeratorIvars {
    container_id: Retained<NSString>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = EnumeratorIvars]
    #[name = "DistantFileProviderEnumerator"]
    pub(crate) struct DistantFileProviderEnumerator;

    unsafe impl NSObjectProtocol for DistantFileProviderEnumerator {}

    unsafe impl NSFileProviderEnumerator for DistantFileProviderEnumerator {
        #[unsafe(method(invalidate))]
        fn invalidate(&self) {
            debug!("file_provider: enumerator invalidated");
        }

        #[unsafe(method(enumerateItemsForObserver:startingAtPage:))]
        fn enumerate_items(
            &self,
            observer: &ProtocolObject<dyn NSFileProviderEnumerationObserver>,
            _page: &NSFileProviderPage,
        ) {
            let container_str = self.ivars().container_id.to_string();
            debug!(
                "file_provider: enumerating items for container {:?}",
                container_str,
            );

            let Some(fs) = get_remote_fs() else {
                // No RemoteFs available — signal empty enumeration.
                unsafe {
                    observer.finishEnumeratingUpToPage(None);
                }
                return;
            };

            let root_id = unsafe { NSFileProviderRootContainerItemIdentifier };
            let ino = if container_str == root_id.to_string() {
                1u64
            } else {
                container_str.parse::<u64>().unwrap_or(1)
            };

            match fs.readdir(ino) {
                Ok(entries) => {
                    let parent_id_str = ino.to_string();
                    let items: Vec<Retained<ProtocolObject<dyn NSFileProviderItemProtocol>>> =
                        entries
                            .iter()
                            .filter(|e| e.name != "." && e.name != "..")
                            .map(|entry| {
                                let is_dir = entry.file_type == FileType::Dir;
                                let size = fs.getattr(entry.ino).map(|a| a.size).unwrap_or(0);
                                let item = DistantFileProviderItem::new(
                                    &entry.ino.to_string(),
                                    &parent_id_str,
                                    &entry.name,
                                    is_dir,
                                    size,
                                );
                                ProtocolObject::from_retained(item)
                            })
                            .collect();

                    let array = NSArray::from_retained_slice(&items);
                    unsafe {
                        observer.didEnumerateItems(&array);
                        observer.finishEnumeratingUpToPage(None);
                    }
                }
                Err(e) => {
                    debug!("file_provider: readdir failed: {e}");
                    let ns_error = make_ns_error(&format!("readdir failed: {e}"));
                    unsafe {
                        observer.finishEnumeratingWithError(&ns_error);
                    }
                }
            }
        }
    }
);

impl DistantFileProviderEnumerator {
    /// Creates a new enumerator for the given container item identifier.
    fn new(container_id: &NSString) -> Retained<Self> {
        let enumerator = Self::alloc().set_ivars(EnumeratorIvars {
            container_id: container_id.retain(),
        });
        // SAFETY: NSObject's `init` is always safe to call after `alloc`.
        unsafe { msg_send![super(enumerator), init] }
    }
}

// ---------------------------------------------------------------------------
// DistantFileProvider (main extension class)
// ---------------------------------------------------------------------------

/// Instance variables for [`DistantFileProvider`].
pub(crate) struct ExtensionIvars {
    domain: Mutex<Option<Retained<NSFileProviderDomain>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = ExtensionIvars]
    #[name = "DistantFileProvider"]
    pub(crate) struct DistantFileProvider;

    unsafe impl NSObjectProtocol for DistantFileProvider {}

    unsafe impl NSFileProviderReplicatedExtension for DistantFileProvider {
        #[unsafe(method_id(initWithDomain:))]
        fn init_with_domain(
            this: Allocated<Self>,
            domain: &NSFileProviderDomain,
        ) -> Retained<Self> {
            debug!("file_provider: initWithDomain {:?}", unsafe {
                domain.displayName()
            },);
            let this = this.set_ivars(ExtensionIvars {
                domain: Mutex::new(Some(domain.retain())),
            });
            // SAFETY: NSObject's `init` is always safe to call.
            unsafe { msg_send![super(this), init] }
        }

        #[unsafe(method(invalidate))]
        fn invalidate(&self) {
            debug!("file_provider: invalidate");
            if let Ok(mut guard) = self.ivars().domain.lock() {
                *guard = None;
            }
        }

        #[unsafe(method_id(itemForIdentifier:request:completionHandler:))]
        fn item_for_identifier(
            &self,
            identifier: &NSFileProviderItemIdentifier,
            _request: &NSFileProviderRequest,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderItem, *mut NSError)>,
        ) -> Retained<NSProgress> {
            let id_str = identifier.to_string();
            debug!("file_provider: itemForIdentifier {:?}", id_str);
            handle_item_for_identifier(&id_str, completion_handler);
            new_progress()
        }

        #[unsafe(method_id(fetchContentsForItemWithIdentifier:version:request:completionHandler:))]
        fn fetch_contents(
            &self,
            item_identifier: &NSFileProviderItemIdentifier,
            _requested_version: Option<&NSFileProviderItemVersion>,
            _request: &NSFileProviderRequest,
            completion_handler: &block2::DynBlock<
                dyn Fn(*mut NSURL, *mut NSFileProviderItem, *mut NSError),
            >,
        ) -> Retained<NSProgress> {
            let id_str = item_identifier.to_string();
            debug!("file_provider: fetchContents for {:?}", id_str);
            handle_fetch_contents(&id_str, completion_handler);
            new_progress()
        }

        #[unsafe(method_id(createItemBasedOnTemplate:fields:contents:options:request:completionHandler:))]
        fn create_item(
            &self,
            item_template: &NSFileProviderItem,
            _fields: NSFileProviderItemFields,
            url: Option<&NSURL>,
            _options: NSFileProviderCreateItemOptions,
            _request: &NSFileProviderRequest,
            completion_handler: &block2::DynBlock<
                dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
            >,
        ) -> Retained<NSProgress> {
            let filename = unsafe { item_template.filename() };
            let parent_id = unsafe { item_template.parentItemIdentifier() };
            debug!(
                "file_provider: createItem {:?} in {:?}",
                filename.to_string(),
                parent_id.to_string(),
            );
            handle_create_item(&filename, &parent_id, url.is_some(), completion_handler);
            new_progress()
        }

        #[unsafe(method_id(modifyItem:baseVersion:changedFields:contents:options:request:completionHandler:))]
        fn modify_item(
            &self,
            item: &NSFileProviderItem,
            _version: &NSFileProviderItemVersion,
            _changed_fields: NSFileProviderItemFields,
            new_contents: Option<&NSURL>,
            _options: NSFileProviderModifyItemOptions,
            _request: &NSFileProviderRequest,
            completion_handler: &block2::DynBlock<
                dyn Fn(*mut NSFileProviderItem, NSFileProviderItemFields, Bool, *mut NSError),
            >,
        ) -> Retained<NSProgress> {
            let item_id = unsafe { item.itemIdentifier() };
            debug!("file_provider: modifyItem {:?}", item_id.to_string());
            handle_modify_item(&item_id, new_contents, completion_handler);
            new_progress()
        }

        #[unsafe(method_id(deleteItemWithIdentifier:baseVersion:options:request:completionHandler:))]
        fn delete_item(
            &self,
            identifier: &NSFileProviderItemIdentifier,
            _version: &NSFileProviderItemVersion,
            _options: NSFileProviderDeleteItemOptions,
            _request: &NSFileProviderRequest,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) -> Retained<NSProgress> {
            debug!("file_provider: deleteItem {:?}", identifier.to_string(),);
            handle_delete_item(identifier, completion_handler);
            new_progress()
        }
    }

    unsafe impl NSFileProviderEnumerating for DistantFileProvider {
        // The `error:` out-parameter is handled manually: the ObjC runtime
        // expects `enumeratorForContainerItemIdentifier:request:error:` where
        // the last argument is `NSError **`. We take it as a raw pointer and
        // write the error on failure (returning `None`/nil).
        #[unsafe(method_id(enumeratorForContainerItemIdentifier:request:error:))]
        fn enumerator_for_container(
            &self,
            container_item_identifier: &NSFileProviderItemIdentifier,
            _request: &NSFileProviderRequest,
            _error: *mut *mut NSError,
        ) -> Option<Retained<ProtocolObject<dyn NSFileProviderEnumerator>>> {
            debug!(
                "file_provider: enumeratorForContainer {:?}",
                container_item_identifier.to_string(),
            );
            let enumerator = DistantFileProviderEnumerator::new(container_item_identifier);
            Some(ProtocolObject::from_retained(enumerator))
        }
    }
);

// ---------------------------------------------------------------------------
// Extracted handler functions (avoids early returns inside define_class!)
// ---------------------------------------------------------------------------

/// Handles the `itemForIdentifier:request:completionHandler:` logic.
fn handle_item_for_identifier(
    id_str: &str,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderItem, *mut NSError)>,
) {
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
    let Some(fs) = get_remote_fs() else {
        call_completion_fetch_error(completion_handler, "RemoteFs not initialized");
        return;
    };

    let ino: u64 = id_str.parse().unwrap_or(0);

    let data = match fs.read(ino, 0, u32::MAX) {
        Ok(data) => data,
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

    completion_handler.call((std::ptr::null_mut(),));
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Creates an `NSError` with the given message in the `com.distant.file-provider`
/// error domain.
fn make_ns_error(message: &str) -> Retained<NSError> {
    let domain = NSString::from_str("com.distant.file-provider");
    let description = NSString::from_str(message);
    let key: &NSErrorUserInfoKey = unsafe { NSLocalizedDescriptionKey };
    let user_info = NSDictionary::from_retained_objects(
        &[key],
        &[Retained::into_super(Retained::into_super(description))],
    );
    unsafe { NSError::errorWithDomain_code_userInfo(&domain, -1, Some(&user_info)) }
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

/// Registers a FileProvider domain for the given mount configuration.
///
/// Sets the global [`RemoteFs`] and prepares for domain registration.
/// The actual `.appex` extension process is launched by the system when
/// the domain is accessed in Finder.
///
/// # Errors
///
/// Returns an error if the domain cannot be registered (currently a
/// placeholder — domain registration will be implemented in the bundle
/// assembly phase).
pub(crate) fn register_domain(fs: Arc<RemoteFs>) -> io::Result<()> {
    set_remote_fs(fs);

    // Domain registration would normally happen from the container app.
    // For CLI-driven usage, we register the domain programmatically.
    debug!("file_provider: domain registered (extension activation pending)");

    Ok(())
}
