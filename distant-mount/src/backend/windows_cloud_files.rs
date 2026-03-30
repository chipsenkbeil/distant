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

use windows::Win32::Storage::CloudFilters as CF;

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
        _ticket: ticket::FetchPlaceholders,
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
                            .mark_in_sync()
                    })
                    .collect();

                // Use CfCreatePlaceholders with a directory handle instead of
                // CfExecute/TRANSFER_PLACEHOLDERS. The Gemini/MS docs indicate
                // CfCreatePlaceholders is the correct API for population.
                use cloud_filter::placeholder_file::BatchCreate;
                let parent_path = match self.relative_path(request.path()) {
                    None => self.mount_point.clone(),
                    Some(ref rel) => self.mount_point.join(rel),
                };
                log::info!("cloud_files: creating placeholders in {:?}", parent_path);
                if let Err(e) = placeholders.create(&parent_path) {
                    log::error!("cloud_files: CfCreatePlaceholders failed: {e}");
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
/// Calls `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS` directly,
/// using `FLAG_NONE` instead of the cloud-filter crate's unconditional
/// `DISABLE_ON_DEMAND_POPULATION` flag.
///
/// The `FetchPlaceholders` ticket stores `{ connection_key: i64, transfer_key: i64 }`
/// as its only fields. We extract them via unsafe cast since the fields are
/// `pub(crate)` in the cloud-filter crate.
#[allow(dead_code)]
fn transfer_placeholders(
    ticket: &ticket::FetchPlaceholders,
    placeholders: &mut [PlaceholderFile],
) -> windows::core::Result<()> {
    // SAFETY: FetchPlaceholders is repr(Rust) with two i64 fields.
    // This matches the struct layout { connection_key: i64, transfer_key: i64 }.
    let keys: &[i64; 2] = unsafe { &*(ticket as *const _ as *const [i64; 2]) };
    let connection_key = keys[0];
    let transfer_key = keys[1];

    let op_info = CF::CF_OPERATION_INFO {
        StructSize: std::mem::size_of::<CF::CF_OPERATION_INFO>() as u32,
        Type: CF::CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS,
        ConnectionKey: CF::CF_CONNECTION_KEY(connection_key),
        TransferKey: transfer_key,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: CF::CF_REQUEST_KEY_DEFAULT as i64,
    };

    let params = CF::CF_OPERATION_PARAMETERS {
        ParamSize: (std::mem::size_of::<CF::CF_OPERATION_PARAMETERS_0_7>()
            + std::mem::offset_of!(CF::CF_OPERATION_PARAMETERS, Anonymous))
            as u32,
        Anonymous: CF::CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: CF::CF_OPERATION_PARAMETERS_0_7 {
                Flags: CF::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_NONE,
                CompletionStatus: windows::Win32::Foundation::STATUS_SUCCESS,
                PlaceholderTotalCount: placeholders.len() as i64,
                PlaceholderArray: if placeholders.is_empty() {
                    std::ptr::null_mut()
                } else {
                    placeholders.as_ptr() as *mut _
                },
                PlaceholderCount: placeholders.len() as u32,
                EntriesProcessed: 0,
            },
        },
    };

    unsafe {
        CF::CfExecute(
            &op_info as *const _,
            &params as *const _ as *mut _,
        )
    }
}

/// Pre-populates the root directory of the sync root with placeholders.
pub(crate) async fn pre_populate(fs: &RemoteFs, mount_point: &Path) -> io::Result<()> {
    // Test: try CfCreatePlaceholders on a non-sync-root folder to verify
    // the API works at all on this system.
    use cloud_filter::placeholder_file::BatchCreate;
    let test_dir = mount_point.parent().unwrap().join("_distant_cf_test");
    let _ = std::fs::create_dir_all(&test_dir);
    log::info!("cloud_files: testing CfCreatePlaceholders on non-sync-root {:?}", test_dir);
    let mut test = vec![
        PlaceholderFile::new("test.txt")
            .metadata(Metadata::file())
            .mark_in_sync(),
    ];
    match test.create(&test_dir) {
        Ok(()) => log::info!("cloud_files: non-sync-root test OK!"),
        Err(e) => log::error!("cloud_files: non-sync-root test ALSO failed: {e}"),
    }
    let _ = std::fs::remove_dir_all(&test_dir);

    // Now try on the actual sync root
    log::info!("cloud_files: testing CfCreatePlaceholders on sync root {:?}", mount_point);
    let mut test2 = vec![
        PlaceholderFile::new("test.txt")
            .metadata(Metadata::file())
            .mark_in_sync(),
    ];
    match test2.create(mount_point) {
        Ok(()) => log::info!("cloud_files: sync root test OK!"),
        Err(e) => {
            log::error!("cloud_files: sync root test failed: {e}");
            return Err(io::Error::other(format!("CfCreatePlaceholders failed: {e}")));
        }
    }

    pre_populate_dir(fs, mount_point, 1).await
}

/// Pre-populate a directory with placeholder files/dirs using CfCreatePlaceholders.
///
/// Called after CfConnectSyncRoot (not from inside a callback).
/// Directories are marked with DISABLE_ON_DEMAND_POPULATION so the OS
/// won't fire FETCH_PLACEHOLDERS callbacks for them.
async fn pre_populate_dir(
    fs: &RemoteFs,
    parent_path: &Path,
    ino: u64,
) -> io::Result<()> {
    use cloud_filter::placeholder_file::BatchCreate;

    let entries = fs.readdir(ino).await?;
    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| e.name != "." && e.name != "..")
        .collect();

    log::info!(
        "cloud_files: pre-populating {} entries in {:?}",
        filtered.len(),
        parent_path,
    );

    let mut placeholders: Vec<PlaceholderFile> = filtered
        .iter()
        .map(|entry| {
            let is_dir = entry.file_type == FileType::Dir;
            let mut p = PlaceholderFile::new(&entry.name).mark_in_sync();
            if is_dir {
                p = p.metadata(Metadata::directory()).has_no_children();
            } else {
                p = p.metadata(Metadata::file());
            }
            p
        })
        .collect();

    if !placeholders.is_empty() {
        placeholders.create(parent_path).map_err(|e| {
            io::Error::other(format!("CfCreatePlaceholders failed: {e}"))
        })?;
    }

    Ok(())
}

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

    // Unregister stale sync root and clean directory to avoid 0x8007017C.
    // The Cloud Filter driver tracks per-directory population state via
    // NTFS reparse points — stale entries cause TRANSFER_PLACEHOLDERS to
    // fail with ERROR_CLOUD_FILE_INVALID_REQUEST.
    if sync_root_id
        .is_registered()
        .map_err(|e| io::Error::other(format!("failed to check registration: {e}")))?
    {
        log::info!("cloud_files: unregistering stale sync root");
        let _ = sync_root_id.unregister();
    }

    // Clean directory contents to remove stale reparse points.
    if mount_point.exists() {
        if let Ok(entries) = std::fs::read_dir(mount_point) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_dir_all(entry.path())
                    .or_else(|_| std::fs::remove_file(entry.path()));
            }
        }
    }

    {
        let mut info = SyncRootInfo::default();
        info.set_display_name("distant - Remote Filesystem");
        info.set_hydration_type(HydrationType::Full);
        info.set_population_type(PopulationType::Full);
        info.set_icon("%SystemRoot%\\system32\\imageres.dll,197");
        info.set_version("0.21.0");
        info.set_path(mount_point).map_err(|e| {
            io::Error::other(format!("failed to set sync root path: {e}"))
        })?;
        let _ = info.set_recycle_bin_uri("https://github.com/chipsenkbeil/distant");

        sync_root_id
            .register(info)
            .map_err(|e| io::Error::other(format!("failed to register sync root: {e}")))?;
    }

    let handler = CloudFilesHandler::new(fs.clone(), mount_point.to_path_buf());
    let handle_clone = handle.clone();

    let connection = Session::new()
        .connect_async(mount_point, handler, move |future| {
            handle_clone.block_on(future);
        })
        .map_err(|e| io::Error::other(format!("failed to connect sync root: {e}")))?;

    // Pre-populate root directory placeholders AFTER connecting.
    // The CloudMirror sample does this — CfCreatePlaceholders must be called
    // after CfConnectSyncRoot, not from inside the FETCH_PLACEHOLDERS callback.
    log::info!("cloud_files: pre-populating root directory");
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
