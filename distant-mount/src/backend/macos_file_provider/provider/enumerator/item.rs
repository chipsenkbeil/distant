use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_file_provider::{NSFileProviderItemIdentifier, NSFileProviderItemProtocol};
use objc2_foundation::{NSObject, NSString};

/// Instance variables for [`DistantFileProviderItem`].
pub struct ItemIvars {
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
    pub struct DistantFileProviderItem;

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
    pub(super) fn new(
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
