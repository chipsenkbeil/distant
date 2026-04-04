use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_file_provider::{
    NSFileProviderItemCapabilities, NSFileProviderItemIdentifier, NSFileProviderItemProtocol,
    NSFileProviderItemVersion,
};
use objc2_foundation::{NSData, NSNumber, NSObject, NSString};
use objc2_uniform_type_identifiers::{UTType, UTTypeData, UTTypeFolder};

/// Instance variables for [`DistantFileProviderItem`].
pub struct ItemIvars {
    identifier: Retained<NSString>,
    parent_identifier: Retained<NSString>,
    filename: Retained<NSString>,
    is_directory: bool,
    readonly: bool,
    size: u64,
    mtime_secs: u64,
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

        #[unsafe(method_id(contentType))]
        fn content_type(&self) -> Retained<UTType> {
            self.compute_content_type()
        }

        #[unsafe(method_id(itemVersion))]
        fn item_version(&self) -> Retained<NSFileProviderItemVersion> {
            self.compute_item_version()
        }

        #[unsafe(method(capabilities))]
        fn capabilities(&self) -> NSFileProviderItemCapabilities {
            self.compute_capabilities()
        }

        #[unsafe(method_id(documentSize))]
        fn document_size(&self) -> Option<Retained<NSNumber>> {
            if self.ivars().is_directory {
                None
            } else {
                Some(NSNumber::new_u64(self.ivars().size))
            }
        }
    }
);

impl DistantFileProviderItem {
    /// Creates a new item with the given metadata.
    pub(crate) fn new(
        identifier: &str,
        parent_identifier: &str,
        filename: &str,
        is_directory: bool,
        readonly: bool,
        size: u64,
        mtime_secs: u64,
    ) -> Retained<Self> {
        let item = Self::alloc().set_ivars(ItemIvars {
            identifier: NSString::from_str(identifier),
            parent_identifier: NSString::from_str(parent_identifier),
            filename: NSString::from_str(filename),
            is_directory,
            readonly,
            size,
            mtime_secs,
        });
        // SAFETY: NSObject's `init` is always safe to call after `alloc`.
        unsafe { msg_send![super(item), init] }
    }

    /// Computes the UTType for this item.
    ///
    /// Extracted from `define_class!` to allow early returns (the macro wraps
    /// method bodies for ObjC autorelease semantics, which is incompatible
    /// with Rust `return` statements).
    fn compute_content_type(&self) -> Retained<UTType> {
        if self.ivars().is_directory {
            // SAFETY: UTTypeFolder is a valid Apple framework constant.
            return unsafe { UTTypeFolder }.retain();
        }

        let ext = self.ivars().filename.pathExtension();
        if ext.length() > 0
            && let Some(ut) = UTType::typeWithFilenameExtension(&ext)
        {
            return ut;
        }

        // SAFETY: UTTypeData is a valid Apple framework constant.
        unsafe { UTTypeData }.retain()
    }

    /// Computes the item version from the file's mtime.
    ///
    /// Uses mtime (seconds since epoch, little-endian bytes) for both the
    /// content and metadata version components. This is cheap and changes
    /// whenever the file is modified.
    fn compute_item_version(&self) -> Retained<NSFileProviderItemVersion> {
        let mtime_bytes = self.ivars().mtime_secs.to_le_bytes();
        let content_version = NSData::with_bytes(&mtime_bytes);
        let metadata_version = NSData::with_bytes(&mtime_bytes);
        unsafe {
            NSFileProviderItemVersion::initWithContentVersion_metadataVersion(
                NSFileProviderItemVersion::alloc(),
                &content_version,
                &metadata_version,
            )
        }
    }

    /// Computes the capabilities for this item based on whether it is a
    /// directory or a file.
    fn compute_capabilities(&self) -> NSFileProviderItemCapabilities {
        if self.ivars().readonly {
            // Readonly mount: only allow reading and enumeration.
            // macOS rejects writes at the OS level before they reach the extension.
            let mut caps = NSFileProviderItemCapabilities::AllowsReading;
            if self.ivars().is_directory {
                caps |= NSFileProviderItemCapabilities::AllowsContentEnumerating;
            }
            return caps;
        }

        if self.ivars().is_directory {
            NSFileProviderItemCapabilities::AllowsReading
                | NSFileProviderItemCapabilities::AllowsContentEnumerating
                | NSFileProviderItemCapabilities::AllowsAddingSubItems
                | NSFileProviderItemCapabilities::AllowsDeleting
                | NSFileProviderItemCapabilities::AllowsRenaming
                | NSFileProviderItemCapabilities::AllowsReparenting
        } else {
            NSFileProviderItemCapabilities::AllowsReading
                | NSFileProviderItemCapabilities::AllowsWriting
                | NSFileProviderItemCapabilities::AllowsDeleting
                | NSFileProviderItemCapabilities::AllowsRenaming
                | NSFileProviderItemCapabilities::AllowsReparenting
        }
    }
}
