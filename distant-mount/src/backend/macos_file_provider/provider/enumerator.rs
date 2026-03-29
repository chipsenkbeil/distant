mod item;

pub(crate) use item::DistantFileProviderItem;

use std::time::SystemTime;

use log::{debug, error, trace};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::{
    NSFileProviderChangeObserver, NSFileProviderEnumerationObserver, NSFileProviderEnumerator,
    NSFileProviderItemProtocol, NSFileProviderPage, NSFileProviderRootContainerItemIdentifier,
    NSFileProviderSyncAnchor,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSString};

use distant_core::protocol::FileType;

use crate::backend::macos_file_provider;

/// Instance variables for [`DistantFileProviderEnumerator`].
pub struct EnumeratorIvars {
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
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let anchor = NSData::with_bytes(&now.to_le_bytes());
            unsafe {
                observer.finishEnumeratingChangesUpToSyncAnchor_moreComing(&anchor, false);
            }
        }

        #[unsafe(method(enumerateItemsForObserver:startingAtPage:))]
        fn enumerate_items(
            &self,
            observer: &ProtocolObject<dyn NSFileProviderEnumerationObserver>,
            _page: &NSFileProviderPage,
        ) {
            let container_str = self.ivars().container_id.to_string();
            trace!(
                "file_provider: enumerating items for container {:?}",
                container_str,
            );

            let Some(rt) = macos_file_provider::get_runtime() else {
                debug!("file_provider: enumerate_items — Runtime not available, returning empty");
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

            let observer = macos_file_provider::UnsafeSendable(observer.retain());

            rt.spawn(move |fs| async move {
                match fs.readdir(ino).await {
                    Ok(entries) => {
                        let parent_id_str = ino.to_string();

                        // Collect metadata with async calls first, then build
                        // ObjC items. This avoids holding Retained<...> (!Send)
                        // across .await points.
                        let mut metadata: Vec<(String, String, bool, u64, u64)> = Vec::new();
                        for entry in entries.iter().filter(|e| e.name != "." && e.name != "..") {
                            let is_dir = entry.file_type == FileType::Dir;
                            let attr = fs.getattr(entry.ino).await;
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
                        let items: Vec<Retained<ProtocolObject<dyn NSFileProviderItemProtocol>>> =
                            metadata
                                .iter()
                                .map(|(ino_str, name, is_dir, size, mtime_secs)| {
                                    let item = DistantFileProviderItem::new(
                                        ino_str,
                                        &parent_id_str,
                                        name,
                                        *is_dir,
                                        *size,
                                        *mtime_secs,
                                    );
                                    ProtocolObject::from_retained(item)
                                })
                                .collect();

                        trace!(
                            "file_provider: enumerate_items for ino={ino} returning {} items",
                            items.len()
                        );
                        let array = NSArray::from_retained_slice(&items);
                        unsafe {
                            observer.didEnumerateItems(&array);
                            observer.finishEnumeratingUpToPage(None);
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
    /// Creates a new enumerator for the given container item identifier.
    pub(super) fn new(container_id: &NSString) -> Retained<Self> {
        let enumerator = Self::alloc().set_ivars(EnumeratorIvars {
            container_id: container_id.retain(),
        });
        // SAFETY: NSObject's `init` is always safe to call after `alloc`.
        unsafe { msg_send![super(enumerator), init] }
    }
}
