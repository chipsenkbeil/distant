use std::any::TypeId;

use super::{Api, Unsupported};

/// Utility class to check if various APIs are supported.
pub struct Checker;

impl Checker {
    /// Returns true if [`FileSystemApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_file_system_support<T>() -> bool
    where
        T: Api,
        T::FileSystem: 'static,
    {
        TypeId::of::<T::FileSystem>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`ProcessApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_process_support<T>() -> bool
    where
        T: Api,
        T::Process: 'static,
    {
        TypeId::of::<T::Process>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`SearchApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_search_support<T>() -> bool
    where
        T: Api,
        T::Search: 'static,
    {
        TypeId::of::<T::Search>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`SystemInfoApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_system_info_support<T>() -> bool
    where
        T: Api,
        T::SystemInfo: 'static,
    {
        TypeId::of::<T::SystemInfo>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`VersionApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_version_support<T>() -> bool
    where
        T: Api,
        T::Version: 'static,
    {
        TypeId::of::<T::Version>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`WatchApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    pub fn has_watch_support<T>() -> bool
    where
        T: Api,
        T::Watch: 'static,
    {
        TypeId::of::<T::Watch>() != TypeId::of::<Unsupported>()
    }
}
