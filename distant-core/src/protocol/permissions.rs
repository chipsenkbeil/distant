use std::cmp;
use std::io;
use std::path::Path;

use bitflags::bitflags;
use ignore::types::TypesBuilder;
use ignore::DirEntry;
use ignore::WalkBuilder;
use log::*;
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
            unix: Some(UnixPermissions::from(std_permissions)),
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
        async fn set_permissions(this: &Permissions, entry: &DirEntry) -> io::Result<()> {
            // If we are on a Unix platform and we have a full permission set, we do not need to
            // retrieve the permissions to modify them and can instead produce a new permission set
            // purely from the Unix permissions
            let permissions = if cfg!(unix) && this.has_complete_unix_permissions() {
                this.unix.unwrap().into()
            } else {
                let mut std_permissions = entry
                    .metadata()
                    .map_err(|x| match x.io_error() {
                        Some(x) => {
                            io::Error::new(x.kind(), format!("(Read permissions failed) {x}"))
                        }
                        None => io::Error::new(
                            io::ErrorKind::Other,
                            format!("(Read permissions failed) {x}"),
                        ),
                    })?
                    .permissions();

                // Apply the readonly flag if we are provided it
                if let Some(readonly) = this.readonly {
                    std_permissions.set_readonly(readonly);
                }

                // Update our unix permissions if we were given new permissions by loading
                // in the current permissions and applying any changes on top of those
                #[cfg(unix)]
                if let Some(permissions) = this.unix {
                    use std::os::unix::prelude::*;
                    let mut current = UnixPermissions::from_unix_mode(std_permissions.mode());
                    current.apply_from(&permissions);
                    std_permissions.set_mode(current.to_unix_mode());
                }

                std_permissions
            };

            if log_enabled!(Level::Trace) {
                let mut output = String::new();
                output.push_str("readonly = ");
                output.push_str(if permissions.readonly() {
                    "true"
                } else {
                    "false"
                });

                #[cfg(unix)]
                {
                    use std::os::unix::prelude::*;
                    output.push_str(&format!(", mode = {:#o}", permissions.mode()));
                }

                trace!("Setting {:?} permissions to ({})", entry.path(), output);
            }

            tokio::fs::set_permissions(entry.path(), permissions)
                .await
                .map_err(|x| io::Error::new(x.kind(), format!("(Set permissions failed) {x}")))
        }

        let walk = WalkBuilder::new(path)
            .follow_links(options.resolve_symlink)
            .max_depth(if options.recursive { None } else { Some(0) })
            .threads(cmp::min(MAXIMUM_THREADS, num_cpus::get()))
            .types(
                TypesBuilder::new()
                    .add_defaults()
                    .build()
                    .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?,
            )
            .skip_stdout(true)
            .build();

        // Process as much as possible and then fail with an error
        let mut errors = Vec::new();
        for entry in walk {
            match entry {
                Ok(entry) => {
                    if let Err(x) = set_permissions(self, &entry).await {
                        errors.push(format!("{:?}: {x}", entry.path()));
                    }
                }
                Err(x) => {
                    errors.push(x.to_string());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                errors
                    .into_iter()
                    .map(|x| format!("* {x}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
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

    /// Returns true if `unix` is populated with a complete permission set as defined by
    /// [`UnixPermissions::is_complete`], otherwise returns false.
    pub fn has_complete_unix_permissions(&self) -> bool {
        if let Some(permissions) = self.unix.as_ref() {
            permissions.is_complete()
        } else {
            false
        }
    }
}

#[cfg(feature = "schemars")]
impl Permissions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Permissions)
    }
}

impl From<std::fs::Permissions> for Permissions {
    fn from(permissions: std::fs::Permissions) -> Self {
        Self {
            readonly: Some(permissions.readonly()),
            #[cfg(unix)]
            unix: Some(UnixPermissions::from(permissions)),
            #[cfg(not(unix))]
            unix: None,
        }
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
impl UnixPermissions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(UnixPermissions)
    }
}

#[cfg(unix)]
impl From<std::fs::Permissions> for UnixPermissions {
    /// Converts [`std::fs::Permissions`] into [`UnixPermissions`] using the `mode`.
    fn from(permissions: std::fs::Permissions) -> Self {
        use std::os::unix::prelude::*;
        Self::from_unix_mode(permissions.mode())
    }
}

#[cfg(unix)]
impl From<UnixPermissions> for std::fs::Permissions {
    /// Converts [`UnixPermissions`] into [`std::fs::Permissions`] using the `mode`.
    fn from(permissions: UnixPermissions) -> Self {
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
