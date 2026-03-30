//! Mount backend trait and platform-specific implementations.

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub mod fuse;

#[cfg(feature = "nfs")]
pub mod nfs;

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
pub mod windows_cloud_files;

// macOS FileProvider backend — requires .appex inside .app bundle.
// See macos_file_provider.rs module docs for architecture details.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub mod macos_file_provider;

use std::error;
use std::fmt;
use std::str::FromStr;

/// Selects which mount backend to use.
///
/// Each variant is only available when its corresponding feature and
/// platform requirements are met. Use [`MountBackend::available_backends`]
/// to discover which backends are compiled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountBackend {
    #[cfg(all(
        feature = "fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ))]
    Fuse,

    #[cfg(feature = "nfs")]
    Nfs,

    #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
    WindowsCloudFiles,

    #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
    MacosFileProvider,
}

impl MountBackend {
    /// Returns a list of all backends available.
    pub fn available_backends() -> &'static [Self] {
        &[
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            Self::Fuse,
            #[cfg(feature = "nfs")]
            Self::Nfs,
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            Self::WindowsCloudFiles,
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            Self::MacosFileProvider,
        ]
    }

    /// Returns whether this backend requires a long-running foreground process.
    ///
    /// NFS and FUSE backends run a server that must stay alive for the mount
    /// to work. FileProvider and Windows Cloud Files are managed by the OS.
    pub fn needs_foreground_process(&self) -> bool {
        match self {
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            Self::Fuse => true,
            #[cfg(feature = "nfs")]
            Self::Nfs => true,
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            Self::WindowsCloudFiles => true,
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            Self::MacosFileProvider => false,
        }
    }

    /// Returns the backend as a str.
    pub fn as_str(&self) -> &'static str {
        match self {
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            Self::Fuse => "fuse",
            #[cfg(feature = "nfs")]
            Self::Nfs => "nfs",
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            Self::WindowsCloudFiles => "windows-cloud-files",
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            Self::MacosFileProvider => "macos-file-provider",
        }
    }
}

impl Default for MountBackend {
    #[allow(clippy::needless_return, unreachable_code)]
    fn default() -> Self {
        #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
        {
            if macos_file_provider::utils::is_running_in_app_bundle() {
                return Self::MacosFileProvider;
            }
        }

        #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
        {
            return Self::WindowsCloudFiles;
        }

        #[cfg(feature = "nfs")]
        {
            return Self::Nfs;
        }

        #[cfg(all(
            feature = "fuse",
            any(target_os = "linux", target_os = "freebsd", target_os = "macos")
        ))]
        {
            return Self::Fuse;
        }

        #[cfg(not(any(
            feature = "nfs",
            all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ),
            all(feature = "macos-file-provider", target_os = "macos"),
            all(feature = "windows-cloud-files", target_os = "windows"),
        )))]
        {
            // This branch is only reached when no mount backends are compiled.
            // The enum is empty in that case, so constructing a value is impossible.
            // The CLI won't expose mount commands without at least one backend feature.
            compile_error!("no mount backend available — enable at least one mount feature")
        }
    }
}

impl fmt::Display for MountBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error returned when parsing an invalid [`MountBackend`] string.
#[derive(Clone, Debug)]
pub struct ParseMountBackendError(String);

impl fmt::Display for ParseMountBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown mount backend {:?}, available: {:?}",
            self.0,
            MountBackend::available_backends()
        )
    }
}

impl error::Error for ParseMountBackendError {}

impl FromStr for MountBackend {
    type Err = ParseMountBackendError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            "fuse" => Ok(Self::Fuse),
            #[cfg(feature = "nfs")]
            "nfs" => Ok(Self::Nfs),
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            "windows-cloud-files" => Ok(Self::WindowsCloudFiles),
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            "macos-file-provider" => Ok(Self::MacosFileProvider),
            _ => Err(ParseMountBackendError(s.to_string())),
        }
    }
}
