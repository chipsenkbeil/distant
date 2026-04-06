mod item;

pub(crate) use item::DistantFileProviderItem;

use std::time::SystemTime;

use log::{debug, error, trace};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::{
    NSFileProviderChangeObserver, NSFileProviderEnumerationObserver, NSFileProviderEnumerator,
    NSFileProviderErrorCode, NSFileProviderItemProtocol, NSFileProviderPage,
    NSFileProviderRootContainerItemIdentifier, NSFileProviderSyncAnchor,
    NSFileProviderTrashContainerItemIdentifier, NSFileProviderWorkingSetContainerItemIdentifier,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSString};

use distant_core::protocol::FileType;

use crate::backend::macos_file_provider;

/// Instance variables for [`DistantFileProviderEnumerator`].
pub struct EnumeratorIvars {
    domain_id: String,
    container_id: Retained<NSString>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = EnumeratorIvars]
    #[name = "DistantFileProviderEnumerator"]
    pub struct DistantFileProviderEnumerator;

    unsafe impl NSObjectProtocol for DistantFileProviderEnumerator {}

    unsafe impl NSFileProviderEnumerator for DistantFileProviderEnumerator {
        #[unsafe(method(invalidate))]
        fn invalidate(&self) {
            debug!("file_provider: enumerator invalidated");
        }

        #[unsafe(method(currentSyncAnchorWithCompletionHandler:))]
        fn current_sync_anchor(
            &self,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSFileProviderSyncAnchor)>,
        ) {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let anchor = NSData::with_bytes(&now.to_le_bytes());
            let raw = Retained::into_raw(anchor);
            completion_handler.call((raw,));
        }

        #[unsafe(method(enumerateChangesForObserver:fromSyncAnchor:))]
        fn enumerate_changes(
            &self,
            observer: &ProtocolObject<dyn NSFileProviderChangeObserver>,
            _sync_anchor: &NSFileProviderSyncAnchor,
        ) {
            // Report the sync anchor as expired so macOS falls back to a
            // full enumerateItems. Without a remote file watcher we cannot
            // track incremental changes. Combined with signalEnumerator on
            // bootstrap, this ensures fresh data on every access.
            let ns_error = macos_file_provider::make_fp_error(
                NSFileProviderErrorCode::SyncAnchorExpired,
                "remote filesystem does not support incremental change tracking",
            );
            unsafe {
                observer.finishEnumeratingWithError(&ns_error);
            }
        }

        #[unsafe(method(enumerateItemsForObserver:startingAtPage:))]
        fn enumerate_items(
            &self,
            observer: &ProtocolObject<dyn NSFileProviderEnumerationObserver>,
            page: &NSFileProviderPage,
        ) {
            let container_str = self.ivars().container_id.to_string();
            trace!(
                "file_provider: enumerating items for container {:?}",
                container_str,
            );

            // Working set and trash containers return empty results immediately.
            let working_set_id =
                unsafe { NSFileProviderWorkingSetContainerItemIdentifier }.to_string();
            let trash_id = unsafe { NSFileProviderTrashContainerItemIdentifier }.to_string();
            if container_str == working_set_id || container_str == trash_id {
                debug!(
                    "file_provider: enumerate_items — returning empty for {:?}",
                    container_str,
                );
                unsafe {
                    observer.finishEnumeratingUpToPage(None);
                }
                return;
            }

            let Some(rt) = macos_file_provider::get_runtime(&self.ivars().domain_id) else {
                if let Some(err_msg) =
                    macos_file_provider::get_bootstrap_error(&self.ivars().domain_id)
                {
                    error!("file_provider: enumerate_items — bootstrap failed: {err_msg}",);
                    let ns_error = macos_file_provider::make_fp_error(
                        NSFileProviderErrorCode::ServerUnreachable,
                        &format!("Bootstrap failed: {err_msg}"),
                    );
                    unsafe {
                        observer.finishEnumeratingWithError(&ns_error);
                    }
                } else {
                    debug!(
                        "file_provider: enumerate_items — Runtime not available, returning empty",
                    );
                    unsafe {
                        observer.finishEnumeratingUpToPage(None);
                    }
                }
                return;
            };

            let root_id = unsafe { NSFileProviderRootContainerItemIdentifier };
            let root_id_str = root_id.to_string();
            let ino = if container_str == root_id_str {
                1u64
            } else {
                container_str.parse::<u64>().unwrap_or(1)
            };

            // Parent identifier: use the root constant string when enumerating
            // the root directory, otherwise the numeric inode string.
            let parent_id_str = if ino == 1 {
                root_id_str
            } else {
                ino.to_string()
            };

            // Parse page offset from the page token. The initial page from
            // the system is a well-known NSData constant (not a u64), so we
            // treat any non-8-byte token as "start from 0".
            let page_offset = parse_page_offset(page);

            let observer = macos_file_provider::UnsafeSendable(observer.retain());

            rt.spawn(move |fs| async move {
                match fs.readdir(ino).await {
                    Ok(entries) => {
                        let filtered: Vec<_> = entries
                            .iter()
                            .filter(|e| e.name != "." && e.name != "..")
                            .collect();
                        let total = filtered.len();
                        let start = (page_offset as usize).min(total);
                        let end = (start + PAGE_SIZE).min(total);
                        let page_entries = &filtered[start..end];

                        // Collect metadata with async calls first, then build
                        // ObjC items. This avoids holding Retained<...> (!Send)
                        // across .await points.
                        let mut metadata: Vec<(String, String, bool, u64, u64)> = Vec::new();
                        for entry in page_entries {
                            let attr = fs.getattr(entry.ino).await;
                            // Use getattr result for type (resolves symlinks)
                            // rather than readdir entry type.
                            let is_dir = attr.as_ref().is_ok_and(|a| a.kind == FileType::Dir);
                            let size = attr.as_ref().map(|a| a.size).unwrap_or(0);
                            let mtime_secs = attr
                                .as_ref()
                                .map(|a| {
                                    a.mtime
                                        .duration_since(SystemTime::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0)
                                })
                                .unwrap_or(0);
                            metadata.push((
                                entry.ino.to_string(),
                                entry.name.clone(),
                                is_dir,
                                size,
                                mtime_secs,
                            ));
                        }

                        // Build ObjC items from collected metadata (no .await).
                        let readonly = fs.is_readonly();
                        let items: Vec<Retained<ProtocolObject<dyn NSFileProviderItemProtocol>>> =
                            metadata
                                .iter()
                                .map(|(ino_str, name, is_dir, size, mtime_secs)| {
                                    let item = DistantFileProviderItem::new(
                                        ino_str,
                                        &parent_id_str,
                                        name,
                                        *is_dir,
                                        readonly,
                                        *size,
                                        *mtime_secs,
                                    );
                                    ProtocolObject::from_retained(item)
                                })
                                .collect();

                        trace!(
                            "file_provider: enumerate_items ino={ino} page={start}..{end}/{total}",
                        );
                        let array = NSArray::from_retained_slice(&items);
                        unsafe {
                            observer.didEnumerateItems(&array);
                        }

                        // Signal next page or end of enumeration.
                        let next_page = if end < total {
                            let token = NSData::with_bytes(&(end as u64).to_le_bytes());
                            Some(token)
                        } else {
                            None
                        };
                        unsafe {
                            observer.finishEnumeratingUpToPage(next_page.as_deref());
                        }
                    }
                    Err(e) => {
                        error!("file_provider: readdir failed for ino={ino}: {e}");
                        let ns_error =
                            macos_file_provider::make_ns_error(&format!("readdir failed: {e}"));
                        unsafe {
                            observer.finishEnumeratingWithError(&ns_error);
                        }
                    }
                }
            });
        }
    }
);

impl DistantFileProviderEnumerator {
    /// Creates a new enumerator for the given domain and container item identifier.
    pub(super) fn new(domain_id: &str, container_id: &NSString) -> Retained<Self> {
        let enumerator = Self::alloc().set_ivars(EnumeratorIvars {
            domain_id: domain_id.to_owned(),
            container_id: container_id.retain(),
        });
        // SAFETY: NSObject's `init` is always safe to call after `alloc`.
        unsafe { msg_send![super(enumerator), init] }
    }
}

/// Number of items to return per enumeration page.
const PAGE_SIZE: usize = 100;

/// Parses a page token (NSData) as a u64 offset.
///
/// The initial page from the system (`NSFileProviderInitialPageSortedByName`
/// etc.) is a well-known NSData constant that won't be exactly 8 bytes, so
/// we treat any non-8-byte token as offset 0.
fn parse_page_offset(page: &NSData) -> u64 {
    let len = page.length();
    if len == 8 {
        let mut buf = [0u8; 8];
        unsafe {
            page.getBytes_length(std::ptr::NonNull::new(buf.as_mut_ptr().cast()).unwrap(), 8);
        }
        u64::from_le_bytes(buf)
    } else {
        0
    }
}
