mod enumerator;

pub(crate) use enumerator::{DistantFileProviderEnumerator, DistantFileProviderItem};

use std::sync::Mutex;

use log::{debug, error, info};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::{
    NSFileProviderCreateItemOptions, NSFileProviderDeleteItemOptions, NSFileProviderDomain,
    NSFileProviderEnumerating, NSFileProviderEnumerator, NSFileProviderItem,
    NSFileProviderItemFields, NSFileProviderItemIdentifier, NSFileProviderItemProtocol,
    NSFileProviderItemVersion, NSFileProviderModifyItemOptions, NSFileProviderReplicatedExtension,
    NSFileProviderRequest,
};
use objc2_foundation::{NSError, NSObject, NSProgress, NSURL};

use super::{
    bootstrap, handle_create_item, handle_delete_item, handle_fetch_contents,
    handle_item_for_identifier, handle_modify_item, new_progress,
};

/// Instance variables for [`DistantFileProvider`].
pub struct ExtensionIvars {
    domain: Mutex<Option<Retained<NSFileProviderDomain>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this type does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = ExtensionIvars]
    #[name = "DistantFileProvider"]
    pub struct DistantFileProvider;

    unsafe impl NSObjectProtocol for DistantFileProvider {}

    unsafe impl NSFileProviderReplicatedExtension for DistantFileProvider {
        #[unsafe(method_id(initWithDomain:))]
        fn init_with_domain(
            this: Allocated<Self>,
            domain: &NSFileProviderDomain,
        ) -> Retained<Self> {
            info!("file_provider: initWithDomain {:?}", unsafe {
                domain.displayName()
            },);
            let domain_id = unsafe { domain.identifier() }.to_string();
            let this = this.set_ivars(ExtensionIvars {
                domain: Mutex::new(Some(domain.retain())),
            });
            // SAFETY: NSObject's `init` is always safe to call.
            let this: Retained<Self> = unsafe { msg_send![super(this), init] };

            // Bootstrap the Runtime from persisted domain metadata.
            // Errors are logged but not fatal — the enumerator handles
            // get_runtime() returning None by signalling empty results.
            match bootstrap(&domain_id) {
                Ok(()) => info!("file_provider: bootstrap succeeded for {domain_id:?}"),
                Err(e) => error!("file_provider: bootstrap FAILED for {domain_id:?}: {e}"),
            }

            this
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
