use bitflags::bitflags;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct SetPermissionsOptions {
    /// Whether or not to exclude symlinks from traversal entirely, meaning that permissions will
    /// not be set on symlinks (usually resolving the symlink and setting the permission of the
    /// referenced file or directory) that are explicitly provided or show up during recursion.
    pub exclude_symlinks: bool,

    /// Whether or not to traverse symlinks when recursively setting permissions. Note that this
    /// does NOT influence setting permissions when encountering a symlink as most platforms will
    /// resolve the symlink before setting permissions.
    pub follow_symlinks: bool,

    /// Whether or not to set the permissions of the file hierarchies rooted in the paths, instead
    /// of just the paths themselves.
    pub recursive: bool,
}

#[cfg(feature = "schemars")]
impl SetPermissionsOptions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SetPermissionsOptions)
    }
}

/// Represents permissions to apply to some path on a remote machine
///
/// When used to set permissions on a file, directory, or symlink,
/// only fields that are set (not `None`) will be applied.
///
/// On `Unix` platforms, this translates directly into the mode that
/// you would find with `chmod`. On all other platforms, this uses the
/// write flags to determine whether or not to set the readonly status.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Permissions {
    /// Represents whether or not owner can read from the file
    pub owner_read: Option<bool>,

    /// Represents whether or not owner can write to the file
    pub owner_write: Option<bool>,

    /// Represents whether or not owner can execute the file
    pub owner_exec: Option<bool>,

    /// Represents whether or not associated group can read from the file
    pub group_read: Option<bool>,

    /// Represents whether or not associated group can write to the file
    pub group_write: Option<bool>,

    /// Represents whether or not associated group can execute the file
    pub group_exec: Option<bool>,

    /// Represents whether or not other can read from the file
    pub other_read: Option<bool>,

    /// Represents whether or not other can write to the file
    pub other_write: Option<bool>,

    /// Represents whether or not other can execute the file
    pub other_exec: Option<bool>,
}

impl Permissions {
    /// Creates a set of [`Permissions`] that indicate readonly status.
    ///
    /// ```
    /// use distant_core::protocol::Permissions;
    ///
    /// let permissions = Permissions::readonly();
    /// assert_eq!(permissions.is_readonly(), Some(true));
    /// assert_eq!(permissions.is_writable(), Some(false));
    /// ```
    pub fn readonly() -> Self {
        Self {
            owner_write: Some(false),
            group_write: Some(false),
            other_write: Some(false),

            owner_read: Some(true),
            group_read: Some(true),
            other_read: Some(true),

            owner_exec: None,
            group_exec: None,
            other_exec: None,
        }
    }
    /// Creates a set of [`Permissions`] that indicate globally writable status.
    ///
    /// ```
    /// use distant_core::protocol::Permissions;
    ///
    /// let permissions = Permissions::writable();
    /// assert_eq!(permissions.is_readonly(), Some(false));
    /// assert_eq!(permissions.is_writable(), Some(true));
    /// ```
    pub fn writable() -> Self {
        Self {
            owner_write: Some(true),
            group_write: Some(true),
            other_write: Some(true),

            owner_read: Some(true),
            group_read: Some(true),
            other_read: Some(true),

            owner_exec: None,
            group_exec: None,
            other_exec: None,
        }
    }

    /// Returns true if the permission set has a value specified for each permission (no `None`
    /// settings).
    pub fn is_complete(&self) -> bool {
        self.owner_read.is_some()
            && self.owner_write.is_some()
            && self.owner_exec.is_some()
            && self.group_read.is_some()
            && self.group_write.is_some()
            && self.group_exec.is_some()
            && self.other_read.is_some()
            && self.other_write.is_some()
            && self.other_exec.is_some()
    }

    /// Returns `true` if permissions represent readonly, `false` if permissions represent
    /// writable, and `None` if no permissions have been set to indicate either status.
    #[inline]
    pub fn is_readonly(&self) -> Option<bool> {
        // Negate the writable status to indicate whether or not readonly
        self.is_writable().map(|x| !x)
    }

    /// Returns `true` if permissions represent ability to write, `false` if permissions represent
    /// inability to write, and `None` if no permissions have been set to indicate either status.
    #[inline]
    pub fn is_writable(&self) -> Option<bool> {
        self.owner_write
            .zip(self.group_write)
            .zip(self.other_write)
            .map(|((owner, group), other)| owner || group || other)
    }

    /// Applies `other` settings to `self`, overwriting any of the permissions in `self` with `other`.
    #[inline]
    pub fn apply_from(&mut self, other: &Self) {
        macro_rules! apply {
            ($key:ident) => {{
                if let Some(value) = other.$key {
                    self.$key = Some(value);
                }
            }};
        }

        apply!(owner_read);
        apply!(owner_write);
        apply!(owner_exec);
        apply!(group_read);
        apply!(group_write);
        apply!(group_exec);
        apply!(other_read);
        apply!(other_write);
        apply!(other_exec);
    }

    /// Applies `self` settings to `other`, overwriting any of the permissions in `other` with
    /// `self`.
    #[inline]
    pub fn apply_to(&self, other: &mut Self) {
        Self::apply_from(other, self)
    }

    /// Converts a Unix `mode` into the permission set.
    pub fn from_unix_mode(mode: u32) -> Self {
        let flags = UnixFilePermissionFlags::from_bits_truncate(mode);
        Self {
            owner_read: Some(flags.contains(UnixFilePermissionFlags::OWNER_READ)),
            owner_write: Some(flags.contains(UnixFilePermissionFlags::OWNER_WRITE)),
            owner_exec: Some(flags.contains(UnixFilePermissionFlags::OWNER_EXEC)),
            group_read: Some(flags.contains(UnixFilePermissionFlags::GROUP_READ)),
            group_write: Some(flags.contains(UnixFilePermissionFlags::GROUP_WRITE)),
            group_exec: Some(flags.contains(UnixFilePermissionFlags::GROUP_EXEC)),
            other_read: Some(flags.contains(UnixFilePermissionFlags::OTHER_READ)),
            other_write: Some(flags.contains(UnixFilePermissionFlags::OTHER_WRITE)),
            other_exec: Some(flags.contains(UnixFilePermissionFlags::OTHER_EXEC)),
        }
    }

    /// Converts to a Unix `mode` from a permission set. For any missing setting, a 0 bit is used.
    pub fn to_unix_mode(&self) -> u32 {
        let mut flags = UnixFilePermissionFlags::empty();

        macro_rules! is_true {
            ($opt:expr) => {{
                $opt.is_some() && $opt.unwrap()
            }};
        }

        if is_true!(self.owner_read) {
            flags.insert(UnixFilePermissionFlags::OWNER_READ);
        }
        if is_true!(self.owner_write) {
            flags.insert(UnixFilePermissionFlags::OWNER_WRITE);
        }
        if is_true!(self.owner_exec) {
            flags.insert(UnixFilePermissionFlags::OWNER_EXEC);
        }

        if is_true!(self.group_read) {
            flags.insert(UnixFilePermissionFlags::GROUP_READ);
        }
        if is_true!(self.group_write) {
            flags.insert(UnixFilePermissionFlags::GROUP_WRITE);
        }
        if is_true!(self.group_exec) {
            flags.insert(UnixFilePermissionFlags::GROUP_EXEC);
        }

        if is_true!(self.other_read) {
            flags.insert(UnixFilePermissionFlags::OTHER_READ);
        }
        if is_true!(self.other_write) {
            flags.insert(UnixFilePermissionFlags::OTHER_WRITE);
        }
        if is_true!(self.other_exec) {
            flags.insert(UnixFilePermissionFlags::OTHER_EXEC);
        }

        flags.bits()
    }
}

#[cfg(feature = "schemars")]
impl Permissions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Permissions)
    }
}

#[cfg(unix)]
impl From<std::fs::Permissions> for Permissions {
    /// Converts [`std::fs::Permissions`] into [`Permissions`] using
    /// [`std::os::unix::fs::PermissionsExt::mode`] to supply the bitset.
    fn from(permissions: std::fs::Permissions) -> Self {
        use std::os::unix::prelude::*;
        Self::from_unix_mode(permissions.mode())
    }
}

#[cfg(not(unix))]
impl From<std::fs::Permissions> for Permissions {
    /// Converts [`std::fs::Permissions`] into [`Permissions`] using the `readonly` flag.
    ///
    /// This will not set executable flags, but will set all read and write flags with write flags
    /// being `false` if `readonly`, otherwise set to `true`.
    fn from(permissions: std::fs::Permissions) -> Self {
        if permissions.readonly() {
            Self::readonly()
        } else {
            Self::writable()
        }
    }
}

#[cfg(unix)]
impl From<Permissions> for std::fs::Permissions {
    /// Converts [`Permissions`] into [`std::fs::Permissions`] using
    /// [`std::os::unix::fs::PermissionsExt::from_mode`].
    fn from(permissions: Permissions) -> Self {
        use std::os::unix::prelude::*;
        std::fs::Permissions::from_mode(permissions.to_unix_mode())
    }
}

bitflags! {
    struct UnixFilePermissionFlags: u32 {
        const OWNER_READ = 0o400;
        const OWNER_WRITE = 0o200;
        const OWNER_EXEC = 0o100;
        const GROUP_READ = 0o40;
        const GROUP_WRITE = 0o20;
        const GROUP_EXEC = 0o10;
        const OTHER_READ = 0o4;
        const OTHER_WRITE = 0o2;
        const OTHER_EXEC = 0o1;
    }
}
