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
use cloud_filter::filter::{Request, SyncFilter};
use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder_file::PlaceholderFile;
use cloud_filter::root::{
    Connection, HydrationType, PopulationType, SecurityId, Session, SyncRootId, SyncRootIdBuilder,
    SyncRootInfo,
};
use cloud_filter::utility::WriteAt;
use log::debug;

use distant_core::protocol::FileType;

use crate::core::Runtime;

/// Wrapper for Cloud Filter ticket types that are thread-safe but `!Send`
/// due to conservative defaults in the `cloud-filter` bindings.
///
/// The Cloud Filter API is documented as callable from arbitrary threads.
struct UnsafeSendable<T>(T);
unsafe impl<T> Send for UnsafeSendable<T> {}
unsafe impl<T> Sync for UnsafeSendable<T> {}
impl<T> std::ops::Deref for UnsafeSendable<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Handler implementing the Cloud Filter API's [`SyncFilter`] trait.
///
/// Delegates all filesystem operations to [`RemoteFs`] via the [`Runtime`]
/// async-to-sync bridge (same pattern as the FUSE backend).
pub(crate) struct CloudFilesHandler {
    rt: Arc<Runtime>,
}

impl CloudFilesHandler {
    pub(crate) fn new(rt: Arc<Runtime>) -> Self {
        Self { rt }
    }
}

impl SyncFilter for CloudFilesHandler {
    fn fetch_data(
        &self,
        request: Request,
        ticket: ticket::FetchData,
        info: info::FetchData,
    ) -> CResult<()> {
        debug!("cloud_files: fetch_data for {:?}", request.path());

        let path = request.path().to_path_buf();
        let path_str = path.to_string_lossy().to_string();
        let range = info.required_file_range();
        let offset = range.start;
        let required_length = range.end - range.start;

        let ticket = UnsafeSendable(ticket);
        self.rt.spawn(move |fs| async move {
            match fs.lookup(1, &path_str).await {
                Ok(attr) => match fs
                    .read(attr.ino, offset as u64, required_length as u32)
                    .await
                {
                    Ok(data) => {
                        if let Err(e) = ticket.write_at(&data, offset as u64) {
                            debug!("cloud_files: write_at failed: {e}");
                        }
                    }
                    Err(e) => debug!("cloud_files: read failed: {e}"),
                },
                Err(e) => debug!("cloud_files: lookup failed for fetch_data: {e}"),
            }
        });

        Ok(())
    }

    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        debug!("cloud_files: fetch_placeholders for {:?}", request.path());

        let path = request.path().to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        let ticket = UnsafeSendable(ticket);
        self.rt.spawn(move |fs| async move {
            let ino = match path_str.as_str() {
                "" | "." | "\\" => 1u64,
                _ => match fs.lookup(1, &path_str).await {
                    Ok(attr) => attr.ino,
                    Err(e) => {
                        debug!("cloud_files: lookup failed: {e}");
                        return;
                    }
                },
            };

            match fs.readdir(ino).await {
                Ok(entries) => {
                    // Collect metadata with async calls first, then build
                    // PlaceholderFile objects (which are !Send) without any
                    // .await between creation and use.
                    let mut metadata: Vec<(String, bool, u64)> = Vec::new();
                    for entry in entries.iter().filter(|e| e.name != "." && e.name != "..") {
                        let is_dir = entry.file_type == FileType::Dir;
                        let attr = fs.getattr(entry.ino).await;
                        let size = attr.as_ref().map(|a| a.size).unwrap_or(0);
                        metadata.push((entry.name.clone(), is_dir, size));
                    }

                    let mut placeholders: Vec<PlaceholderFile> = metadata
                        .iter()
                        .map(|(name, is_dir, size)| {
                            let mut p = PlaceholderFile::new(name).mark_in_sync();
                            if *is_dir {
                                p = p.metadata(Metadata::directory()).overwrite();
                            } else {
                                p = p.metadata(Metadata::file().size(*size));
                            }
                            p
                        })
                        .collect();

                    if let Err(e) = ticket.pass_with_placeholder(&mut placeholders) {
                        debug!("cloud_files: pass_with_placeholder failed: {e}");
                    }
                }
                Err(e) => debug!("cloud_files: readdir failed: {e}"),
            }
        });

        Ok(())
    }

    fn deleted(&self, request: Request, _info: info::Deleted) {
        debug!("cloud_files: deleted {:?}", request.path());

        let path = request.path().to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        self.rt.spawn(move |fs| async move {
            if let Ok(attr) = fs.lookup(1, &path_str).await {
                if attr.kind == FileType::Dir {
                    let _ = fs.rmdir(1, &path_str).await;
                } else {
                    let _ = fs.unlink(1, &path_str).await;
                }
            }
        });
    }

    fn renamed(&self, request: Request, info: info::Renamed) {
        let src = info.source_path().to_string_lossy().to_string();
        let dst = request.path().to_string_lossy().to_string();
        debug!("cloud_files: renamed {src:?} -> {dst:?}");

        self.rt.spawn(move |fs| async move {
            if fs.lookup(1, &src).await.is_ok() {
                let _ = fs.rename(1, &src, 1, &dst).await;
            }
        });
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
pub(crate) fn mount(
    rt: Arc<Runtime>,
    mount_point: &Path,
) -> io::Result<Connection<CloudFilesHandler>> {
    let sync_root_id = build_sync_root_id()?;

    if !sync_root_id
        .is_registered()
        .map_err(|e| io::Error::other(format!("failed to check registration: {e}")))?
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

    let handler = CloudFilesHandler::new(rt);

    Session::new()
        .connect(mount_point, handler)
        .map_err(|e| io::Error::other(format!("failed to connect sync root: {e}")))
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
