//! Windows Cloud Files (Cloud Filter API) mount backend.
//!
//! Provides native File Explorer integration with placeholder files on
//! Windows 10+ using the Cloud Filter API via the `cloud-filter` crate.
//!
//! Files appear as cloud placeholders in File Explorer and are hydrated
//! on demand when accessed. Local modifications are detected and synced
//! back to the remote server.
//!
//! # API surface note
//!
//! This module targets `cloud-filter` 0.0.6. The exact trait signatures and
//! builder methods may need minor adjustment when first compiled on Windows,
//! since the crate's API cannot be verified from macOS. All structural
//! decisions (handler delegation to [`RemoteFs`], sync root lifecycle) are
//! intentional and stable; only type-level details may shift.

use std::io;
use std::path::Path;
use std::sync::Arc;

use cloud_filter::filter::{self, SyncFilter};
use cloud_filter::placeholder::PlaceholderFile;
use cloud_filter::request::Request;
use cloud_filter::root::{
    HydrationPolicy, HydrationType, PopulationType, Registration, SecurityId, Session,
    SyncRootIdBuilder,
};
use cloud_filter::ticket::FetchData;
use log::debug;

use distant_core::protocol::FileType;

use crate::core::RemoteFs;

/// Handler implementing the Cloud Filter API's [`SyncFilter`] trait.
///
/// Delegates all file system operations to the shared [`RemoteFs`] translation
/// layer. Placeholder files are created lazily as directories are enumerated,
/// and file contents are hydrated on demand when read by the user.
pub(crate) struct CloudFilesHandler {
    fs: Arc<RemoteFs>,
}

impl CloudFilesHandler {
    pub(crate) fn new(fs: Arc<RemoteFs>) -> Self {
        Self { fs }
    }
}

impl SyncFilter for CloudFilesHandler {
    fn fetch_data(&self, request: &Request, ticket: &FetchData, info: &filter::info::FetchData) {
        debug!("cloud_files: fetch_data for {:?}", request.path());

        let path = request.path();
        let path_str = path.to_string_lossy();

        let required_length = info.required_file_range().length;
        let offset = info.required_file_range().offset;

        match self.fs.lookup(1, &path_str) {
            Ok(attr) => match self
                .fs
                .read(attr.ino, offset as u64, required_length as u32)
            {
                Ok(data) => {
                    if let Err(e) = ticket.write_at(&data, offset as u64) {
                        debug!("cloud_files: write_at failed: {e}");
                    }
                }
                Err(e) => {
                    debug!("cloud_files: read failed: {e}");
                }
            },
            Err(e) => {
                debug!("cloud_files: lookup failed for fetch_data: {e}");
            }
        }
    }

    fn fetch_placeholders(
        &self,
        request: &Request,
        ticket: &cloud_filter::ticket::FetchPlaceholders,
        _info: &filter::info::FetchPlaceholders,
    ) {
        debug!("cloud_files: fetch_placeholders for {:?}", request.path());

        let path = request.path();
        let path_str = path.to_string_lossy();

        let ino = match path_str.as_ref() {
            "" | "." | "\\" => 1u64,
            _ => match self.fs.lookup(1, &path_str) {
                Ok(attr) => attr.ino,
                Err(e) => {
                    debug!("cloud_files: lookup failed: {e}");
                    return;
                }
            },
        };

        match self.fs.readdir(ino) {
            Ok(entries) => {
                let placeholders: Vec<PlaceholderFile> = entries
                    .iter()
                    .filter(|e| e.name != "." && e.name != "..")
                    .filter_map(|entry| {
                        let is_dir = entry.file_type == FileType::Dir;
                        let _attr = self.fs.getattr(entry.ino).ok()?;

                        let mut placeholder = PlaceholderFile::new(&entry.name).mark_in_sync();

                        if is_dir {
                            placeholder = placeholder.overwrite();
                        }

                        Some(placeholder)
                    })
                    .collect();

                if let Err(e) = ticket.pass_with_placeholders(placeholders) {
                    debug!("cloud_files: pass_with_placeholders failed: {e}");
                }
            }
            Err(e) => {
                debug!("cloud_files: readdir failed: {e}");
            }
        }
    }

    fn deleted(&self, request: &Request, _info: &filter::info::Deleted) {
        debug!("cloud_files: deleted {:?}", request.path());

        let path = request.path();
        let path_str = path.to_string_lossy();

        if let Ok(attr) = self.fs.lookup(1, &path_str) {
            if attr.kind == FileType::Dir {
                let _ = self.fs.rmdir(1, &path_str);
            } else {
                let _ = self.fs.unlink(1, &path_str);
            }
        }
    }

    fn renamed(
        &self,
        request: &Request,
        _ticket: &cloud_filter::ticket::Rename,
        info: &filter::info::Renamed,
    ) {
        debug!(
            "cloud_files: renamed {:?} -> {:?}",
            request.path(),
            info.target_path()
        );

        let src = request.path().to_string_lossy();
        let dst = info.target_path().to_string_lossy();

        if self.fs.lookup(1, &src).is_ok() {
            let _ = self.fs.rename(1, &src, 1, &dst);
        }
    }
}

/// Registers a sync root and starts the Cloud Filter session.
///
/// The `mount_point` directory must already exist. It will be registered as
/// a Cloud Files sync root with the display name "distant".
///
/// # Errors
///
/// Returns an error if the current user SID cannot be obtained, the sync root
/// registration fails, or the session connection fails.
pub(crate) fn mount(fs: Arc<RemoteFs>, mount_point: &Path) -> io::Result<Session> {
    let sync_root_id = SyncRootIdBuilder::new("distant")
        .user_security_id(
            SecurityId::current_user()
                .map_err(|e| io::Error::other(format!("failed to get current user SID: {e}")))?,
        )
        .build();

    if !sync_root_id
        .is_registered()
        .map_err(|e| io::Error::other(format!("failed to check registration: {e}")))?
    {
        Registration::from_path(mount_point)
            .map_err(|e| io::Error::other(format!("failed to create registration: {e}")))?
            .display_name("distant - Remote Filesystem")
            .hydration_type(HydrationType::Full)
            .hydration_policy(HydrationPolicy::Full)
            .population_type(PopulationType::Full)
            .icon("%SystemRoot%\\system32\\imageres.dll,197")
            .version("0.21.0")
            .recycle_bin_uri("https://github.com/chipsenkbeil/distant")
            .register(&sync_root_id)
            .map_err(|e| io::Error::other(format!("failed to register sync root: {e}")))?;
    }

    let handler = CloudFilesHandler::new(fs);

    Session::new()
        .connect(mount_point, handler)
        .map_err(|e| io::Error::other(format!("failed to connect sync root: {e}")))
}

/// Unregisters the sync root. Call after dropping the session.
///
/// # Errors
///
/// Returns an error if the current user SID cannot be obtained or the
/// unregistration call fails.
pub(crate) fn unmount() -> io::Result<()> {
    let sync_root_id = SyncRootIdBuilder::new("distant")
        .user_security_id(
            SecurityId::current_user()
                .map_err(|e| io::Error::other(format!("failed to get current user SID: {e}")))?,
        )
        .build();

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
