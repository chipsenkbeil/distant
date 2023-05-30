use bitflags::bitflags;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Represents permissions to apply to some path on a remote machine
///
/// When used to set permissions on a file, directory, or symlink,
/// only fields that are set (not `None`) will be applied.
///
/// On `Unix` platforms, this translates directly into the mode that
/// you would find with `chmod`. On all other platforms, this uses the
/// write flags to determine whether or not to set the readonly status.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Represents whether or not owner can read from the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_read: Option<bool>,

    /// Represents whether or not owner can write to the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_write: Option<bool>,

    /// Represents whether or not owner can execute the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_exec: Option<bool>,

    /// Represents whether or not associated group can read from the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_read: Option<bool>,

    /// Represents whether or not associated group can write to the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_write: Option<bool>,

    /// Represents whether or not associated group can execute the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_exec: Option<bool>,

    /// Represents whether or not other can read from the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_read: Option<bool>,

    /// Represents whether or not other can write to the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_write: Option<bool>,

    /// Represents whether or not other can execute the file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_exec: Option<bool>,
}

impl Permissions {
    /// Creates a set of [`Permissions`] that indicate readonly status.
    ///
    /// ```
    /// use distant_protocol::Permissions;
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
    /// use distant_protocol::Permissions;
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
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// let permissions = Permissions {
    ///     owner_write: Some(true),
    ///     group_write: Some(false),
    ///     other_write: Some(true),
    ///     owner_read: Some(false),
    ///     group_read: Some(true),
    ///     other_read: Some(false),
    ///     owner_exec: Some(true),
    ///     group_exec: Some(false),
    ///     other_exec: Some(true),
    /// };
    /// assert!(permissions.is_complete());
    /// ```
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
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// assert_eq!(
    ///     Permissions { owner_write: Some(true), ..Default::default() }.is_readonly(),
    ///     Some(false)
    /// );
    ///
    /// assert_eq!(
    ///     Permissions { owner_write: Some(false), ..Default::default() }.is_readonly(),
    ///     Some(true)
    /// );
    ///
    /// assert_eq!(
    ///     Permissions { ..Default::default() }.is_writable(),
    ///     None
    /// );
    /// ```
    #[inline]
    pub fn is_readonly(&self) -> Option<bool> {
        // Negate the writable status to indicate whether or not readonly
        self.is_writable().map(|x| !x)
    }

    /// Returns `true` if permissions represent ability to write, `false` if permissions represent
    /// inability to write, and `None` if no permissions have been set to indicate either status.
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// assert_eq!(
    ///     Permissions { owner_write: Some(true), ..Default::default() }.is_writable(),
    ///     Some(true)
    /// );
    ///
    /// assert_eq!(
    ///     Permissions { owner_write: Some(false), ..Default::default() }.is_writable(),
    ///     Some(false)
    /// );
    ///
    /// assert_eq!(
    ///     Permissions { ..Default::default() }.is_writable(),
    ///     None
    /// );
    /// ```
    #[inline]
    pub fn is_writable(&self) -> Option<bool> {
        match (self.owner_write, self.group_write, self.other_write) {
            (None, None, None) => None,
            (owner, group, other) => {
                Some(owner.unwrap_or(false) || group.unwrap_or(false) || other.unwrap_or(false))
            }
        }
    }

    /// Applies `other` settings to `self`, overwriting any of the permissions in `self` with `other`.
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// let mut a = Permissions {
    ///     owner_read: Some(true),
    ///     owner_write: Some(false),
    ///     owner_exec: None,
    ///     ..Default::default()
    /// };
    ///
    /// let b = Permissions {
    ///     owner_read: Some(false),
    ///     owner_write: None,
    ///     owner_exec: Some(true),
    ///     ..Default::default()
    /// };
    ///
    /// a.apply_from(&b);
    ///
    /// assert_eq!(a, Permissions {
    ///     owner_read: Some(false),
    ///     owner_write: Some(false),
    ///     owner_exec: Some(true),
    ///     ..Default::default()
    /// });
    /// ```
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
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// let a = Permissions {
    ///     owner_read: Some(true),
    ///     owner_write: Some(false),
    ///     owner_exec: None,
    ///     ..Default::default()
    /// };
    ///
    /// let mut b = Permissions {
    ///     owner_read: Some(false),
    ///     owner_write: None,
    ///     owner_exec: Some(true),
    ///     ..Default::default()
    /// };
    ///
    /// a.apply_to(&mut b);
    ///
    /// assert_eq!(b, Permissions {
    ///     owner_read: Some(true),
    ///     owner_write: Some(false),
    ///     owner_exec: Some(true),
    ///     ..Default::default()
    /// });
    /// ```
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
    ///
    /// ```
    /// use distant_protocol::Permissions;
    ///
    /// assert_eq!(Permissions {
    ///     owner_read: Some(true),
    ///     owner_write: Some(true),
    ///     owner_exec: Some(true),
    ///     group_read: Some(true),
    ///     group_write: Some(true),
    ///     group_exec: Some(true),
    ///     other_read: Some(true),
    ///     other_write: Some(true),
    ///     other_exec: Some(true),
    /// }.to_unix_mode(), 0o777);
    ///
    /// assert_eq!(Permissions {
    ///     owner_read: Some(true),
    ///     owner_write: Some(false),
    ///     owner_exec: Some(false),
    ///     group_read: Some(true),
    ///     group_write: Some(false),
    ///     group_exec: Some(false),
    ///     other_read: Some(true),
    ///     other_write: Some(false),
    ///     other_exec: Some(false),
    /// }.to_unix_mode(), 0o444);
    ///
    /// assert_eq!(Permissions {
    ///     owner_exec: Some(true),
    ///     group_exec: Some(true),
    ///     other_exec: Some(true),
    ///     ..Default::default()
    /// }.to_unix_mode(), 0o111);
    /// ```
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_be_able_to_serialize_minimal_permissions_to_json() {
        let permissions = Permissions {
            owner_read: None,
            owner_write: None,
            owner_exec: None,
            group_read: None,
            group_write: None,
            group_exec: None,
            other_read: None,
            other_write: None,
            other_exec: None,
        };

        let value = serde_json::to_value(permissions).unwrap();
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn should_be_able_to_serialize_full_permissions_to_json() {
        let permissions = Permissions {
            owner_read: Some(true),
            owner_write: Some(false),
            owner_exec: Some(true),
            group_read: Some(false),
            group_write: Some(true),
            group_exec: Some(false),
            other_read: Some(true),
            other_write: Some(false),
            other_exec: Some(true),
        };

        let value = serde_json::to_value(permissions).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "owner_read": true,
                "owner_write": false,
                "owner_exec": true,
                "group_read": false,
                "group_write": true,
                "group_exec": false,
                "other_read": true,
                "other_write": false,
                "other_exec": true,
            })
        );
    }

    #[test]
    fn should_be_able_to_deserialize_minimal_permissions_from_json() {
        let value = serde_json::json!({});

        let permissions: Permissions = serde_json::from_value(value).unwrap();
        assert_eq!(
            permissions,
            Permissions {
                owner_read: None,
                owner_write: None,
                owner_exec: None,
                group_read: None,
                group_write: None,
                group_exec: None,
                other_read: None,
                other_write: None,
                other_exec: None,
            }
        );
    }

    #[test]
    fn should_be_able_to_deserialize_full_permissions_from_json() {
        let value = serde_json::json!({
            "owner_read": true,
            "owner_write": false,
            "owner_exec": true,
            "group_read": false,
            "group_write": true,
            "group_exec": false,
            "other_read": true,
            "other_write": false,
            "other_exec": true,
        });

        let permissions: Permissions = serde_json::from_value(value).unwrap();
        assert_eq!(
            permissions,
            Permissions {
                owner_read: Some(true),
                owner_write: Some(false),
                owner_exec: Some(true),
                group_read: Some(false),
                group_write: Some(true),
                group_exec: Some(false),
                other_read: Some(true),
                other_write: Some(false),
                other_exec: Some(true),
            }
        );
    }

    #[test]
    fn should_be_able_to_serialize_minimal_permissions_to_msgpack() {
        let permissions = Permissions {
            owner_read: None,
            owner_write: None,
            owner_exec: None,
            group_read: None,
            group_write: None,
            group_exec: None,
            other_read: None,
            other_write: None,
            other_exec: None,
        };

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&permissions).unwrap();
    }

    #[test]
    fn should_be_able_to_serialize_full_permissions_to_msgpack() {
        let permissions = Permissions {
            owner_read: Some(true),
            owner_write: Some(false),
            owner_exec: Some(true),
            group_read: Some(true),
            group_write: Some(false),
            group_exec: Some(true),
            other_read: Some(true),
            other_write: Some(false),
            other_exec: Some(true),
        };

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&permissions).unwrap();
    }

    #[test]
    fn should_be_able_to_deserialize_minimal_permissions_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or preventing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&Permissions {
            owner_read: None,
            owner_write: None,
            owner_exec: None,
            group_read: None,
            group_write: None,
            group_exec: None,
            other_read: None,
            other_write: None,
            other_exec: None,
        })
        .unwrap();

        let permissions: Permissions = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            permissions,
            Permissions {
                owner_read: None,
                owner_write: None,
                owner_exec: None,
                group_read: None,
                group_write: None,
                group_exec: None,
                other_read: None,
                other_write: None,
                other_exec: None,
            }
        );
    }

    #[test]
    fn should_be_able_to_deserialize_full_permissions_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or preventing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&Permissions {
            owner_read: Some(true),
            owner_write: Some(false),
            owner_exec: Some(true),
            group_read: Some(true),
            group_write: Some(false),
            group_exec: Some(true),
            other_read: Some(true),
            other_write: Some(false),
            other_exec: Some(true),
        })
        .unwrap();

        let permissions: Permissions = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            permissions,
            Permissions {
                owner_read: Some(true),
                owner_write: Some(false),
                owner_exec: Some(true),
                group_read: Some(true),
                group_write: Some(false),
                group_exec: Some(true),
                other_read: Some(true),
                other_write: Some(false),
                other_exec: Some(true),
            }
        );
    }
}
