//! Windows Cloud Files (Cloud Filter API) mount backend.
//!
//! Provides native File Explorer integration with placeholder files on
//! Windows 10+ using the Cloud Filter API via the `cloud-filter` crate.
//!
//! Files appear as cloud placeholders in File Explorer and are hydrated
//! on demand when accessed.

use std::io;
use std::path::Path;
use std::sync::Arc;

use cloud_filter::error::CResult;
use cloud_filter::filter::info;
use cloud_filter::filter::ticket;
use cloud_filter::filter::{Filter, Request};
use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder_file::PlaceholderFile;
use cloud_filter::root::{
    HydrationType, PopulationType, SecurityId, Session, SyncRootId, SyncRootIdBuilder,
    SyncRootInfo,
};
use cloud_filter::utility::WriteAt;
use log::debug;

use distant_core::protocol::FileType;

use crate::core::RemoteFs;

/// Async handler implementing the Cloud Filter API's [`Filter`] trait.
///
/// Uses `Session::connect_async` which bridges async callbacks to the
/// Cloud Filter's synchronous thread model via a `block_on` closure.
/// No `Send` bounds are required on the futures.
pub(crate) struct CloudFilesHandler {
    fs: Arc<RemoteFs>,
    mount_point: std::path::PathBuf,
}

impl CloudFilesHandler {
    pub(crate) fn new(fs: Arc<RemoteFs>, mount_point: std::path::PathBuf) -> Self {
        Self { fs, mount_point }
    }

    /// Converts a Cloud Filter absolute path to a relative path within the
    /// remote filesystem. Returns `None` for the sync root itself (inode 1).
    fn relative_path(&self, full_path: impl AsRef<Path>) -> Option<String> {
        full_path
            .as_ref()
            .strip_prefix(&self.mount_point)
            .ok()
            .and_then(|rel| {
                let s = rel.to_string_lossy();
                if s.is_empty() {
                    None // root directory
                } else {
                    Some(s.to_string())
                }
            })
    }
}

impl Filter for CloudFilesHandler {
    async fn fetch_data(
        &self,
        request: Request,
        ticket: ticket::FetchData,
        info: info::FetchData,
    ) -> CResult<()> {
        log::info!("cloud_files: fetch_data for {:?}", request.path());

        let range = info.required_file_range();
        let offset = range.start;
        let length = range.end - range.start;

        let rel = self.relative_path(request.path());
        let path_str = match &rel {
            Some(s) => s.as_str(),
            None => return Ok(()), // root dir, no file content
        };

        if let Ok(attr) = self.fs.lookup(1, path_str).await {
            if let Ok(data) = self.fs.read(attr.ino, offset, length as u32).await {
                if let Err(e) = ticket.write_at(&data, offset) {
                    debug!("cloud_files: write_at failed: {e}");
                }
            }
        }

        Ok(())
    }

    async fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        log::info!("cloud_files: fetch_placeholders for {:?}", request.path());

        let ino = match self.relative_path(request.path()) {
            None => 1u64, // root directory
            Some(ref path_str) => match self.fs.lookup(1, path_str).await {
                Ok(attr) => attr.ino,
                Err(e) => {
                    debug!("cloud_files: lookup failed: {e}");
                    return Ok(());
                }
            },
        };

        log::info!("cloud_files: readdir for ino={ino}");
        match self.fs.readdir(ino).await {
            Ok(entries) => {
                let filtered: Vec<_> = entries
                    .iter()
                    .filter(|e| e.name != "." && e.name != "..")
                    .collect();
                log::info!(
                    "cloud_files: readdir returned {} entries ({} after filter)",
                    entries.len(),
                    filtered.len(),
                );

                // Build placeholders from readdir info only (no per-file
                // getattr calls). Size is set to 0 — the actual size is
                // fetched when the file is hydrated via fetch_data.
                // This avoids timeout from slow per-entry round trips.
                let mut placeholders: Vec<PlaceholderFile> = filtered
                    .iter()
                    .map(|entry| {
                        let is_dir = entry.file_type == FileType::Dir;
                        let meta = if is_dir {
                            Metadata::directory()
                        } else {
                            Metadata::file()
                        };
                        PlaceholderFile::new(&entry.name)
                            .metadata(meta)
                            .overwrite()
                            .mark_in_sync()
                    })
                    .collect();

                log::info!("cloud_files: passing {} placeholders", placeholders.len());
                if let Err(e) = ticket.pass_with_placeholder(&mut placeholders) {
                    log::error!("cloud_files: pass_with_placeholder failed: {e}");
                }
            }
            Err(e) => debug!("cloud_files: readdir failed: {e}"),
        }

        Ok(())
    }

    async fn deleted(&self, request: Request, _info: info::Deleted) {
        log::info!("cloud_files: deleted {:?}", request.path());

        let path_str = match self.relative_path(request.path()) {
            Some(s) => s,
            None => return,
        };

        if let Ok(attr) = self.fs.lookup(1, &path_str).await {
            if attr.kind == FileType::Dir {
                let _ = self.fs.rmdir(1, &path_str).await;
            } else {
                let _ = self.fs.unlink(1, &path_str).await;
            }
        }
    }

    async fn renamed(&self, request: Request, info: info::Renamed) {
        let src = self
            .relative_path(&info.source_path())
            .unwrap_or_default();
        let dst = self.relative_path(request.path()).unwrap_or_default();
        debug!("cloud_files: renamed {src:?} -> {dst:?}");

        if !src.is_empty() && !dst.is_empty() && self.fs.lookup(1, &src).await.is_ok() {
            let _ = self.fs.rename(1, &src, 1, &dst).await;
        }
    }
}

/// Builds the sync root ID for distant.
fn build_sync_root_id() -> io::Result<SyncRootId> {
    Ok(SyncRootIdBuilder::new("distant")
        .user_security_id(
            SecurityId::current_user()
                .map_err(|e| io::Error::other(format!("failed to get current user SID: {e}")))?,
        )
        .build())
}

/// Registers a sync root and starts the Cloud Filter session.
///
/// Uses `connect_async` with Tokio's `block_on` to bridge async callbacks
/// to the Cloud Filter's synchronous callback threads. The futures run on
/// the callback thread (no `Send` required), while the Tokio runtime
/// handles the actual async I/O.
/// Registers sync root, connects, and returns a guard that keeps the
/// connection alive. Drop the guard to disconnect.
pub(crate) fn mount(
    handle: tokio::runtime::Handle,
    fs: Arc<RemoteFs>,
    mount_point: &Path,
) -> io::Result<Box<dyn std::any::Any + Send>> {
    let sync_root_id = build_sync_root_id()?;

    // Always unregister first to clear stale state from previous mounts.
    // Without this, pass_with_placeholder fails with 0x8007017C.
    if sync_root_id
        .is_registered()
        .map_err(|e| io::Error::other(format!("failed to check registration: {e}")))?
    {
        log::info!("cloud_files: unregistering stale sync root");
        let _ = sync_root_id.unregister();
    }

    {
        let mut info = SyncRootInfo::default();
        info.set_display_name("distant - Remote Filesystem");
        info.set_hydration_type(HydrationType::Full);
        info.set_population_type(PopulationType::Full);
        info.set_icon("%SystemRoot%\\system32\\imageres.dll,197");
        info.set_version("0.21.0");
        let _ = info.set_path(mount_point);
        let _ = info.set_recycle_bin_uri("https://github.com/chipsenkbeil/distant");

        sync_root_id
            .register(info)
            .map_err(|e| io::Error::other(format!("failed to register sync root: {e}")))?;
    }

    let handler = CloudFilesHandler::new(fs, mount_point.to_path_buf());

    let connection = Session::new()
        .connect_async(mount_point, handler, move |future| {
            log::info!("cloud_files: block_on callback invoked");
            handle.block_on(future);
            log::info!("cloud_files: block_on callback completed");
        })
        .map_err(|e| io::Error::other(format!("failed to connect sync root: {e}")))?;

    Ok(Box::new(connection))
}

/// Unregisters the sync root. Call after dropping the session.
pub(crate) fn unmount() -> io::Result<()> {
    let sync_root_id = build_sync_root_id()?;

    if sync_root_id
        .is_registered()
        .map_err(|e| io::Error::other(format!("failed to check registration: {e}")))?
    {
        sync_root_id
            .unregister()
            .map_err(|e| io::Error::other(format!("failed to unregister sync root: {e}")))?;
    }

    Ok(())
}
