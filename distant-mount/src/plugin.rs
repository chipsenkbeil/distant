//! Mount plugin implementations for each backend.
//!
//! Each plugin wraps the backend-specific mount logic behind the
//! [`MountPlugin`](distant_core::plugin::MountPlugin) trait, allowing the
//! manager to orchestrate mounts without knowing backend details.

use std::future::Future;
use std::io;
use std::pin::Pin;

use distant_core::Channel;
use distant_core::plugin::{MountHandle as MountHandleTrait, MountPlugin};
use distant_core::protocol::MountConfig;
use tokio::runtime::Handle;

use crate::core::MountHandle as ConcreteMountHandle;

/// Wraps the concrete [`MountHandle`](crate::core::handle::MountHandle) to
/// implement the [`MountHandle`](distant_core::plugin::MountHandle) trait.
struct MountHandleWrapper {
    inner: Option<ConcreteMountHandle>,
    mount_point: String,
}

impl MountHandleTrait for MountHandleWrapper {
    fn unmount(&mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> {
        Box::pin(async {
            if let Some(handle) = self.inner.take() {
                handle.unmount().await
            } else {
                Ok(())
            }
        })
    }

    fn mount_point(&self) -> &str {
        &self.mount_point
    }
}

// ---------------------------------------------------------------------------
// NFS
// ---------------------------------------------------------------------------

#[cfg(feature = "nfs")]
pub struct NfsMountPlugin;

#[cfg(feature = "nfs")]
impl MountPlugin for NfsMountPlugin {
    fn name(&self) -> &str {
        "nfs"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let handle =
                crate::mount(Handle::current(), channel, config, crate::MountBackend::Nfs).await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Some(handle),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

// ---------------------------------------------------------------------------
// FUSE
// ---------------------------------------------------------------------------

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub struct FuseMountPlugin;

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
impl MountPlugin for FuseMountPlugin {
    fn name(&self) -> &str {
        "fuse"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let handle = crate::mount(
                Handle::current(),
                channel,
                config,
                crate::MountBackend::Fuse,
            )
            .await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Some(handle),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

// ---------------------------------------------------------------------------
// macOS FileProvider
// ---------------------------------------------------------------------------

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub struct FileProviderMountPlugin;

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
impl MountPlugin for FileProviderMountPlugin {
    fn name(&self) -> &str {
        "macos-file-provider"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            // FileProvider doesn't have a traditional mount point — macOS manages
            // the CloudStorage directory. We'll discover it after registration.
            let mount_point = String::new();

            let handle = crate::mount(
                Handle::current(),
                channel,
                config,
                crate::MountBackend::MacosFileProvider,
            )
            .await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Some(handle),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

// ---------------------------------------------------------------------------
// Windows Cloud Files
// ---------------------------------------------------------------------------

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
pub struct CloudFilesMountPlugin;

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
impl MountPlugin for CloudFilesMountPlugin {
    fn name(&self) -> &str {
        "windows-cloud-files"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let handle = crate::mount(
                Handle::current(),
                channel,
                config,
                crate::MountBackend::WindowsCloudFiles,
            )
            .await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Some(handle),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}
