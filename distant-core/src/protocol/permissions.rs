use std::cmp;
use std::io;
use std::path::Path;

use bitflags::bitflags;
use ignore::types::TypesBuilder;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

const MAXIMUM_THREADS: usize = 12;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct SetPermissionsOptions {
    /// Whether or not to set the permissions of the file hierarchies rooted in the paths, instead
    /// of just the paths themselves
    pub recursive: bool,

    /// Whether or not to resolve the pathes to the underlying file/directory prior to setting the
    /// permissions
    pub resolve_symlink: bool,
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Permissions {
    /// Whether or not the file/directory/symlink is marked as unwriteable
    pub readonly: Option<bool>,

    /// Represents permissions that are specific to a unix remote machine
    pub unix: Option<UnixPermissions>,
}

impl Permissions {
    pub async fn read(path: impl AsRef<Path>, resolve_symlink: bool) -> io::Result<Self> {
        let std_permissions = Self::read_std_permissions(path, resolve_symlink).await?;

        Ok(Self {
            readonly: Some(std_permissions.readonly()),
            #[cfg(unix)]
            unix: Some({
                use std::os::unix::prelude::*;
                crate::protocol::UnixPermissions::from(std_permissions.mode())
            }),
            #[cfg(not(unix))]
            unix: None,
        })
    }

    /// Sets the permissions for the specified `path`.
    ///
    /// If `resolve_symlink` is true, will resolve the path to the underlying file/directory prior
    /// to attempting to set the permissions.
    ///
    /// If `recursive` is true, will apply permissions to all
    ///
    /// When used to set permissions on a file, directory, or symlink, only fields that are set
    /// (not `None`) will be applied.
    pub async fn write(
        &self,
        path: impl AsRef<Path>,
        options: SetPermissionsOptions,
    ) -> io::Result<()> {
        macro_rules! set_permissions {
            ($path:expr) => {{
                let mut std_permissions =
                    Self::read_std_permissions($path, options.resolve_symlink).await?;

                // Apply the readonly flag if we are provided it
                if let Some(readonly) = self.readonly {
                    std_permissions.set_readonly(readonly);
                }

                // Update our unix permissions if we were given new permissions by loading
                // in the current permissions and applying any changes on top of those
                #[cfg(unix)]
                if let Some(permissions) = self.unix {
                    use std::os::unix::prelude::*;
                    let mut current = UnixPermissions::from(std_permissions.mode());
                    current.merge(&permissions);
                    std_permissions.set_mode(current.into());
                }

                tokio::fs::set_permissions($path, std_permissions).await?;
            }};
        }

        if !options.recursive {
            set_permissions!(path.as_ref());
            Ok(())
        } else {
            let walk = WalkBuilder::new(path)
                .follow_links(options.resolve_symlink)
                .threads(cmp::min(MAXIMUM_THREADS, num_cpus::get()))
                .types(
                    TypesBuilder::new()
                        .add_defaults()
                        .build()
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?,
                )
                .skip_stdout(true)
                .build();

            for result in walk {
                let entry = result.map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                set_permissions!(entry.path());
            }

            Ok(())
        }
    }

    /// Reads [`std::fs::Permissions`] from `path`.
    ///
    /// If `resolve_symlink` is true, will resolve the path to the underlying file/directory prior
    /// to attempting to read the permissions.
    async fn read_std_permissions(
        path: impl AsRef<Path>,
        resolve_symlink: bool,
    ) -> io::Result<std::fs::Permissions> {
        Ok(if resolve_symlink {
            tokio::fs::metadata(path.as_ref()).await?.permissions()
        } else {
            tokio::fs::symlink_metadata(path.as_ref())
                .await?
                .permissions()
        })
    }
}

#[cfg(feature = "schemars")]
impl Permissions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Permissions)
    }
}

/// Represents unix-specific permissions about some path on a remote machine
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UnixPermissions {
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

impl UnixPermissions {
    /// Merges `other` with `self`, overwriting any of the permissions in `self` with `other`.
    pub fn merge(&mut self, other: &Self) {
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
}

#[cfg(feature = "schemars")]
impl UnixPermissions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(UnixPermissions)
    }
}

impl From<u32> for UnixPermissions {
    /// Create from a unix mode bitset
    fn from(mode: u32) -> Self {
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
}

impl From<UnixPermissions> for u32 {
    /// Convert to a unix mode bitset
    fn from(metadata: UnixPermissions) -> Self {
        let mut flags = UnixFilePermissionFlags::empty();

        macro_rules! is_true {
            ($opt:expr) => {{
                $opt.is_some() && $opt.unwrap()
            }};
        }

        if is_true!(metadata.owner_read) {
            flags.insert(UnixFilePermissionFlags::OWNER_READ);
        }
        if is_true!(metadata.owner_write) {
            flags.insert(UnixFilePermissionFlags::OWNER_WRITE);
        }
        if is_true!(metadata.owner_exec) {
            flags.insert(UnixFilePermissionFlags::OWNER_EXEC);
        }

        if is_true!(metadata.group_read) {
            flags.insert(UnixFilePermissionFlags::GROUP_READ);
        }
        if is_true!(metadata.group_write) {
            flags.insert(UnixFilePermissionFlags::GROUP_WRITE);
        }
        if is_true!(metadata.group_exec) {
            flags.insert(UnixFilePermissionFlags::GROUP_EXEC);
        }

        if is_true!(metadata.other_read) {
            flags.insert(UnixFilePermissionFlags::OTHER_READ);
        }
        if is_true!(metadata.other_write) {
            flags.insert(UnixFilePermissionFlags::OTHER_WRITE);
        }
        if is_true!(metadata.other_exec) {
            flags.insert(UnixFilePermissionFlags::OTHER_EXEC);
        }

        flags.bits()
    }
}

impl UnixPermissions {
    pub fn is_readonly(self) -> bool {
        macro_rules! is_true {
            ($opt:expr) => {{
                $opt.is_some() && $opt.unwrap()
            }};
        }

        !(is_true!(self.owner_read) || is_true!(self.group_read) || is_true!(self.other_read))
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
