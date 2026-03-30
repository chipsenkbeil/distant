//! Windows Cloud Files (Cloud Filter API) mount backend.
//!
//! Provides native File Explorer integration with placeholder files on
//! Windows 10+ using the Cloud Filter API via the `windows` crate directly.
//!
//! Files appear as cloud placeholders in File Explorer and are hydrated
//! on demand when accessed. Callbacks are C-style function pointers that
//! access shared state through module-level statics.

use std::any::Any;
use std::collections::HashSet;
use std::ffi::c_void;
use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use distant_core::protocol::FileType;
use tokio::runtime::Handle;
use typed_path::Utf8TypedPath;
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::Storage::CloudFilters;
use windows::Win32::Storage::FileSystem;
use windows::core::PCWSTR;

use crate::core;

/// Fixed GUID identifying the distant cloud provider.
const PROVIDER_GUID: windows::core::GUID = windows::core::GUID::from_values(
    0xd157a417,
    0x1234,
    0x5678,
    [0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78],
);

/// Tokio runtime handle for bridging async operations inside sync callbacks.
///
/// Set once during [`mount`] before any callbacks can fire. Callbacks use
/// `handle.block_on(...)` to execute async remote filesystem operations.
static TOKIO_HANDLE: OnceLock<Handle> = OnceLock::new();

/// Shared remote filesystem instance accessible from callbacks.
///
/// Set once during [`mount`]. Callbacks read from this to service
/// fetch, placeholder, and notification requests.
static REMOTE_FS: OnceLock<Arc<core::RemoteFs>> = OnceLock::new();

/// Local mount point path for the sync root.
///
/// Set once during [`mount`]. Used by callbacks to convert absolute
/// paths from the Cloud Filter API into relative paths for the remote
/// filesystem.
static MOUNT_POINT: OnceLock<PathBuf> = OnceLock::new();

/// Tracks directories that have been populated via FETCH_PLACEHOLDERS.
///
/// Once a directory's placeholders have been transferred, subsequent
/// FETCH_PLACEHOLDERS callbacks for the same directory receive an empty
/// response to avoid `ERROR_ALREADY_EXISTS` (0x800700B7).
static POPULATED_DIRS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Converts a `&str` to a null-terminated UTF-16 wide string.
fn to_wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Converts a `&Path` to a null-terminated UTF-16 wide string.
fn path_to_wide(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Converts a [`windows::core::Error`] into a [`std::io::Error`] preserving
/// the original HRESULT as a raw OS error code.
fn win_err_to_io(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 as i32)
}

/// Registers a sync root, connects with Cloud Filter callbacks, and returns
/// a guard that keeps the connection alive.
///
/// The returned `Box<dyn Any + Send>` holds a [`ConnectionGuard`] wrapping
/// the `CF_CONNECTION_KEY`. When the guard is dropped, the sync root is
/// disconnected via `CfDisconnectSyncRoot`.
///
/// # Errors
///
/// Returns an error if the sync root cannot be registered or connected,
/// or if the global statics have already been initialized (double mount).
pub(crate) fn mount(
    handle: Handle,
    fs: Arc<core::RemoteFs>,
    mount_point: &Path,
) -> io::Result<Box<dyn Any + Send>> {
    TOKIO_HANDLE
        .set(handle)
        .map_err(|_| io::Error::other("TOKIO_HANDLE already initialized"))?;
    REMOTE_FS
        .set(fs)
        .map_err(|_| io::Error::other("REMOTE_FS already initialized"))?;
    MOUNT_POINT
        .set(mount_point.to_path_buf())
        .map_err(|_| io::Error::other("MOUNT_POINT already initialized"))?;
    let _ = POPULATED_DIRS.set(Mutex::new(HashSet::new()));

    let sync_root_id = build_sync_root_id(mount_point);
    log::info!(
        "cloud_files: registering sync root {sync_root_id:?} at {}",
        mount_point.display()
    );

    let wide_path = path_to_wide(mount_point);
    let pcwstr = PCWSTR(wide_path.as_ptr());

    // Unregister any stale sync root at this path. This is idempotent —
    // if no sync root is registered, the call fails silently.
    // SAFETY: pcwstr points to a valid null-terminated wide string that
    // lives for the duration of this call.
    let _ = unsafe { CloudFilters::CfUnregisterSyncRoot(pcwstr) };

    // Clean directory contents to remove stale reparse points left by a
    // prior registration that may have been interrupted.
    if mount_point.exists() {
        if let Ok(entries) = std::fs::read_dir(mount_point) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_dir_all(entry.path())
                    .or_else(|_| std::fs::remove_file(entry.path()));
            }
        }
    }

    let sync_root_id_bytes = sync_root_id.as_bytes();
    let provider_name = to_wide("distant");
    let version_str = format!("{}\0", env!("CARGO_PKG_VERSION"));
    let provider_version = to_wide(&version_str);

    let registration = CloudFilters::CF_SYNC_REGISTRATION {
        StructSize: mem::size_of::<CloudFilters::CF_SYNC_REGISTRATION>() as u32,
        ProviderName: PCWSTR(provider_name.as_ptr()),
        ProviderVersion: PCWSTR(provider_version.as_ptr()),
        SyncRootIdentity: sync_root_id_bytes.as_ptr() as *const c_void,
        SyncRootIdentityLength: sync_root_id_bytes.len() as u32,
        FileIdentity: std::ptr::null(),
        FileIdentityLength: 0,
        ProviderId: PROVIDER_GUID,
    };

    let policies = CloudFilters::CF_SYNC_POLICIES {
        StructSize: mem::size_of::<CloudFilters::CF_SYNC_POLICIES>() as u32,
        Hydration: CloudFilters::CF_HYDRATION_POLICY {
            Primary: CloudFilters::CF_HYDRATION_POLICY_FULL,
            Modifier: CloudFilters::CF_HYDRATION_POLICY_MODIFIER_NONE,
        },
        Population: CloudFilters::CF_POPULATION_POLICY {
            Primary: CloudFilters::CF_POPULATION_POLICY_FULL,
            Modifier: CloudFilters::CF_POPULATION_POLICY_MODIFIER_NONE,
        },
        InSync: CloudFilters::CF_INSYNC_POLICY_TRACK_ALL,
        HardLink: CloudFilters::CF_HARDLINK_POLICY_NONE,
        PlaceholderManagement: CloudFilters::CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
    };

    // SAFETY: pcwstr points to a valid null-terminated wide string.
    // registration and policies are valid structs that live for the
    // duration of this call. CF_REGISTER_FLAG_UPDATE allows re-registration
    // if the sync root already exists with different settings.
    unsafe {
        CloudFilters::CfRegisterSyncRoot(
            pcwstr,
            &registration,
            &policies,
            CloudFilters::CF_REGISTER_FLAG_UPDATE,
        )
        .map_err(win_err_to_io)?;
    }

    log::info!("cloud_files: sync root registered successfully");

    let callbacks = [
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_FETCH_DATA,
            Callback: Some(on_fetch_data),
        },
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_CANCEL_FETCH_DATA,
            Callback: Some(on_cancel_fetch_data),
        },
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS,
            Callback: Some(on_fetch_placeholders),
        },
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_NOTIFY_DELETE,
            Callback: Some(on_notify_delete),
        },
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_NOTIFY_RENAME,
            Callback: Some(on_notify_rename),
        },
        CloudFilters::CF_CALLBACK_REGISTRATION {
            Type: CloudFilters::CF_CALLBACK_TYPE_NONE,
            Callback: None,
        },
    ];

    // SAFETY: pcwstr points to a valid null-terminated wide string.
    // callbacks is a valid array terminated with CF_CALLBACK_TYPE_NONE.
    // The callback function pointers match the CF_CALLBACK signature.
    // CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO and CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH
    // request additional context in callback info structs.
    let connection_key = unsafe {
        CloudFilters::CfConnectSyncRoot(
            pcwstr,
            callbacks.as_ptr(),
            None,
            CloudFilters::CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO
                | CloudFilters::CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH,
        )
        .map_err(win_err_to_io)?
    };

    log::info!("cloud_files: connected to sync root");

    Ok(Box::new(ConnectionGuard { connection_key }))
}

/// Guard that disconnects the Cloud Filter sync root on drop.
///
/// Holds the `CF_CONNECTION_KEY` returned by `CfConnectSyncRoot` and
/// explicitly calls `CfDisconnectSyncRoot` when dropped to ensure
/// clean teardown with logging.
struct ConnectionGuard {
    connection_key: CloudFilters::CF_CONNECTION_KEY,
}

// SAFETY: CF_CONNECTION_KEY wraps an i64 and has no thread affinity.
// The Cloud Filter API allows disconnect from any thread.
unsafe impl Send for ConnectionGuard {}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        log::info!("cloud_files: disconnecting sync root");

        // SAFETY: connection_key was obtained from a successful
        // CfConnectSyncRoot call. Disconnecting is safe as long as
        // the key is valid, which it is — we own it exclusively.
        unsafe {
            let _ = CloudFilters::CfDisconnectSyncRoot(self.connection_key);
        }
    }
}

/// Pre-populates the root directory of the sync root with placeholders.
///
/// Must be called after [`mount`] (i.e., after `CfConnectSyncRoot`), not
/// from inside a callback. Creates placeholder entries for all files and
/// directories in the remote root so they appear immediately in Explorer.
///
/// # Errors
///
/// Returns an error if the remote filesystem cannot be read or if
/// placeholder creation fails.
#[allow(dead_code)]
pub(crate) async fn pre_populate(fs: &core::RemoteFs, mount_point: &Path) -> io::Result<()> {
    log::info!(
        "cloud_files: pre-populating root at {}",
        mount_point.display()
    );

    let entries = fs.readdir(1).await?;
    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| e.name != "." && e.name != "..")
        .collect();

    log::info!(
        "cloud_files: root has {} entries to populate",
        filtered.len()
    );

    if filtered.is_empty() {
        return Ok(());
    }

    // Build wide strings and placeholder structs. We must keep the wide
    // string buffers alive until after CfCreatePlaceholders returns, so
    // collect them into parallel Vecs.
    let mut wide_names: Vec<Vec<u16>> = Vec::with_capacity(filtered.len());
    let mut identity_strings: Vec<String> = Vec::with_capacity(filtered.len());

    for entry in &filtered {
        wide_names.push(to_wide(&entry.name));
        identity_strings.push(entry.name.clone());
    }

    let mut placeholders: Vec<CloudFilters::CF_PLACEHOLDER_CREATE_INFO> =
        Vec::with_capacity(filtered.len());

    for (i, entry) in filtered.iter().enumerate() {
        let file_attributes = if entry.file_type == FileType::Dir {
            FileSystem::FILE_ATTRIBUTE_DIRECTORY.0
        } else {
            FileSystem::FILE_ATTRIBUTE_NORMAL.0
        };

        let fs_metadata = CloudFilters::CF_FS_METADATA {
            BasicInfo: FileSystem::FILE_BASIC_INFO {
                CreationTime: 0,
                LastAccessTime: 0,
                LastWriteTime: 0,
                ChangeTime: 0,
                FileAttributes: file_attributes,
            },
            FileSize: 0,
        };

        let identity_bytes = identity_strings[i].as_bytes();

        placeholders.push(CloudFilters::CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(wide_names[i].as_ptr()),
            FsMetadata: fs_metadata,
            FileIdentity: identity_bytes.as_ptr() as *const c_void,
            FileIdentityLength: identity_bytes.len() as u32,
            Flags: CloudFilters::CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            Result: windows::core::HRESULT(0),
            CreateUsn: 0,
        });
    }

    let wide_mount = path_to_wide(mount_point);

    // SAFETY: wide_mount is a valid null-terminated wide string.
    // placeholders is a valid array of CF_PLACEHOLDER_CREATE_INFO structs.
    // All referenced wide strings and identity byte slices are alive in
    // wide_names and identity_strings respectively.
    let result = unsafe {
        CloudFilters::CfCreatePlaceholders(
            PCWSTR(wide_mount.as_ptr()),
            &mut placeholders,
            CloudFilters::CF_CREATE_FLAG_NONE,
            None,
        )
    };

    match result {
        Ok(()) => {
            log::info!(
                "cloud_files: created {} root placeholders",
                placeholders.len()
            );
        }
        Err(e) if e.code().0 == 0x8007017Cu32 as i32 => {
            // ERROR_CLOUD_FILE_IN_USE or similar — the OS already has
            // placeholders. This is not fatal; the FETCH_PLACEHOLDERS
            // callback will handle on-demand population.
            log::warn!(
                "cloud_files: CfCreatePlaceholders returned 0x8007017C, \
                 relying on FETCH_PLACEHOLDERS callback"
            );
        }
        Err(e) => return Err(win_err_to_io(e)),
    }

    Ok(())
}

/// Unregisters the sync root. Call after dropping the connection guard.
///
/// # Errors
///
/// Returns an error if the sync root cannot be unregistered.
pub(crate) fn unmount() -> io::Result<()> {
    let mount_point = MOUNT_POINT.get().ok_or_else(|| {
        io::Error::other("cloud_files: MOUNT_POINT not initialized, cannot unmount")
    })?;

    let sync_root_id = build_sync_root_id(mount_point);
    log::info!("cloud_files: unregistering sync root {sync_root_id:?}");

    let wide_path = path_to_wide(mount_point);

    // SAFETY: wide_path is a valid null-terminated wide string that lives
    // for the duration of this call.
    unsafe { CloudFilters::CfUnregisterSyncRoot(PCWSTR(wide_path.as_ptr())).map_err(win_err_to_io) }
}

/// Extracts the `NormalizedPath` field from a `CF_CALLBACK_INFO` pointer
/// as a lossy UTF-8 string for logging purposes.
///
/// Returns `"<unknown>"` if the pointer is null or the path cannot be read.
///
/// # Safety
///
/// The caller must ensure `info` is either null or points to a valid
/// `CF_CALLBACK_INFO` struct populated by the Cloud Filter driver.
unsafe fn normalized_path_from_info(info: *const CloudFilters::CF_CALLBACK_INFO) -> String {
    if info.is_null() {
        return "<unknown>".to_string();
    }

    // SAFETY: caller guarantees info is a valid pointer.
    let info_ref = unsafe { &*info };
    let normalized = info_ref.NormalizedPath;
    if normalized.is_null() {
        return "<unknown>".to_string();
    }

    // SAFETY: NormalizedPath is populated by the OS as a valid wide string.
    unsafe {
        normalized
            .to_string()
            .unwrap_or_else(|_| "<invalid-utf16>".to_string())
    }
}

/// Callback invoked when the OS needs file content (hydration).
///
/// Reads the requested byte range from the remote filesystem and transfers
/// it back via `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_DATA`.
///
/// # Safety
///
/// Called by the Windows Cloud Filter driver. `info` and `params` must be
/// valid pointers to structures populated by the OS. This function is only
/// invoked on callback threads managed by the Cloud Filter API.
unsafe extern "system" fn on_fetch_data(
    info: *const CloudFilters::CF_CALLBACK_INFO,
    params: *const CloudFilters::CF_CALLBACK_PARAMETERS,
) {
    if info.is_null() || params.is_null() {
        log::error!("cloud_files: on_fetch_data called with null pointer");
        return;
    }

    // SAFETY: info and params were checked non-null above and are guaranteed
    // valid by the Cloud Filter driver contract.
    let info_ref = unsafe { &*info };
    let params_ref = unsafe { &*params };
    let path = unsafe { normalized_path_from_info(info) };

    // SAFETY: This callback is FETCH_DATA, so the FetchData union variant
    // is the active field per the Cloud Filter driver contract.
    let fetch_data = unsafe { &params_ref.Anonymous.FetchData };
    let offset = fetch_data.RequiredFileOffset;
    let length = fetch_data.RequiredLength;

    log::info!("cloud_files: on_fetch_data for {path}, offset={offset}, length={length}");

    let handle = match TOKIO_HANDLE.get() {
        Some(h) => h,
        None => {
            log::error!("cloud_files: TOKIO_HANDLE not initialized in callback");
            return;
        }
    };
    let fs = match REMOTE_FS.get() {
        Some(f) => f,
        None => {
            log::error!("cloud_files: REMOTE_FS not initialized in callback");
            return;
        }
    };

    // Extract the relative path from FileIdentity (set during placeholder
    // creation) or fall back to NormalizedPath.
    let file_identity = extract_file_identity(info_ref);

    // Single block_on: resolve the path and read the data in one async block.
    let read_result = handle.block_on(async {
        let attr = resolve_relative_path(fs, &file_identity).await?;
        fs.read(attr.ino, offset as u64, length as u32).await
    });

    match read_result {
        Ok(data) => transfer_data_response(info_ref, offset, &data, true),
        Err(e) => {
            log::error!("cloud_files: fetch_data failed for {file_identity}: {e}");
            transfer_data_response(info_ref, offset, &[], false);
        }
    }
}

/// Callback invoked when the OS cancels an in-progress fetch.
///
/// # Safety
///
/// Called by the Windows Cloud Filter driver. `info` and `params` must be
/// valid pointers to structures populated by the OS.
#[allow(dead_code)]
unsafe extern "system" fn on_cancel_fetch_data(
    info: *const CloudFilters::CF_CALLBACK_INFO,
    _params: *const CloudFilters::CF_CALLBACK_PARAMETERS,
) {
    if info.is_null() {
        log::error!("cloud_files: on_cancel_fetch_data called with null info pointer");
        return;
    }

    // SAFETY: info was checked non-null above.
    let path = unsafe { normalized_path_from_info(info) };
    log::debug!("cloud_files: on_cancel_fetch_data for {path}");
}

/// Callback invoked when the OS needs directory listing (placeholder population).
///
/// Reads the directory contents from the remote filesystem and transfers
/// placeholders via `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS`
/// and the `DISABLE_ON_DEMAND_POPULATION` flag. This flag tells the Cloud
/// Filter driver the directory is fully populated, preventing re-requests.
///
/// # Safety
///
/// Called by the Windows Cloud Filter driver. `info` and `params` must be
/// valid pointers to structures populated by the OS. This function is only
/// invoked on callback threads managed by the Cloud Filter API.
unsafe extern "system" fn on_fetch_placeholders(
    info: *const CloudFilters::CF_CALLBACK_INFO,
    _params: *const CloudFilters::CF_CALLBACK_PARAMETERS,
) {
    if info.is_null() {
        log::error!("cloud_files: on_fetch_placeholders called with null info pointer");
        return;
    }

    // SAFETY: info was checked non-null above and is guaranteed valid
    // by the Cloud Filter driver contract.
    let info_ref = unsafe { &*info };
    let path = unsafe { normalized_path_from_info(info) };
    log::info!("cloud_files: on_fetch_placeholders for {path}");

    let disable_flag =
        CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION;

    let handle = match TOKIO_HANDLE.get() {
        Some(h) => h,
        None => {
            log::error!("cloud_files: TOKIO_HANDLE not initialized in callback");
            transfer_placeholders_response(info_ref, &mut [], disable_flag);
            return;
        }
    };
    let fs = match REMOTE_FS.get() {
        Some(f) => f,
        None => {
            log::error!("cloud_files: REMOTE_FS not initialized in callback");
            transfer_placeholders_response(info_ref, &mut [], disable_flag);
            return;
        }
    };

    // Determine the relative path for the directory being queried.
    // FileIdentity stores the relative path from the sync root (set during
    // placeholder creation). For the root directory it may be empty.
    let rel_path = extract_file_identity(info_ref);

    // Skip directories that have already been populated to avoid
    // ERROR_ALREADY_EXISTS (0x800700B7) from duplicate placeholders.
    if let Some(dirs) = POPULATED_DIRS.get() {
        if let Ok(set) = dirs.lock() {
            if set.contains(&rel_path) {
                log::debug!("cloud_files: directory already populated, skipping: {rel_path:?}");
                transfer_placeholders_response(info_ref, &mut [], disable_flag);
                return;
            }
        }
    }

    // Single block_on call: resolve the directory, read its entries, and
    // fetch file sizes concurrently. This avoids multiple sequential
    // block_on calls on the callback thread.
    let dir_result = handle.block_on(fetch_dir_entries(fs, &rel_path));

    let dir_entries = match dir_result {
        Ok(entries) if entries.is_empty() => {
            transfer_placeholders_response(info_ref, &mut [], disable_flag);
            if let Some(dirs) = POPULATED_DIRS.get() {
                if let Ok(mut set) = dirs.lock() {
                    set.insert(rel_path);
                }
            }
            return;
        }
        Ok(entries) => entries,
        Err(e) => {
            log::error!("cloud_files: fetch_dir_entries failed for {rel_path:?}: {e}");
            transfer_placeholders_response(info_ref, &mut [], disable_flag);
            return;
        }
    };

    // Build wide string buffers and identity strings. These must outlive
    // the placeholder array since it holds raw pointers into them.
    let mut wide_names: Vec<Vec<u16>> = Vec::with_capacity(dir_entries.len());
    let mut identity_strings: Vec<String> = Vec::with_capacity(dir_entries.len());

    for (name, _, _) in &dir_entries {
        wide_names.push(to_wide(name));

        let identity = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{rel_path}/{name}")
        };
        identity_strings.push(identity);
    }

    let mut placeholders: Vec<CloudFilters::CF_PLACEHOLDER_CREATE_INFO> =
        Vec::with_capacity(dir_entries.len());

    for (i, (_, file_type, file_size)) in dir_entries.iter().enumerate() {
        let is_dir = *file_type == FileType::Dir;
        let file_attributes = if is_dir {
            FileSystem::FILE_ATTRIBUTE_DIRECTORY.0
        } else {
            FileSystem::FILE_ATTRIBUTE_NORMAL.0
        };

        let identity_bytes = identity_strings[i].as_bytes();

        placeholders.push(CloudFilters::CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(wide_names[i].as_ptr()),
            FsMetadata: CloudFilters::CF_FS_METADATA {
                BasicInfo: FileSystem::FILE_BASIC_INFO {
                    CreationTime: 0,
                    LastAccessTime: 0,
                    LastWriteTime: 0,
                    ChangeTime: 0,
                    FileAttributes: file_attributes,
                },
                FileSize: *file_size,
            },
            FileIdentity: identity_bytes.as_ptr() as *const c_void,
            FileIdentityLength: identity_bytes.len() as u32,
            Flags: CloudFilters::CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            Result: windows::core::HRESULT(0),
            CreateUsn: 0,
        });
    }

    // Transfer placeholders to the OS via CfExecute with the DISABLE flag
    // to mark the directory as fully populated, preventing re-requests.
    transfer_placeholders_response(info_ref, &mut placeholders, disable_flag);

    // Mark this directory as populated so subsequent callbacks return
    // an empty response immediately.
    if let Some(dirs) = POPULATED_DIRS.get() {
        if let Ok(mut set) = dirs.lock() {
            set.insert(rel_path);
        }
    }
}

/// Callback invoked when a placeholder file or directory is about to be
/// deleted locally. Propagates the deletion to the remote filesystem and
/// responds with ACK_DELETE to allow or deny the operation.
///
/// # Safety
///
/// Called by the Windows Cloud Filter driver. `info` and `params` must be
/// valid pointers to structures populated by the OS.
unsafe extern "system" fn on_notify_delete(
    info: *const CloudFilters::CF_CALLBACK_INFO,
    params: *const CloudFilters::CF_CALLBACK_PARAMETERS,
) {
    if info.is_null() || params.is_null() {
        log::error!("cloud_files: on_notify_delete called with null pointer");
        return;
    }

    // SAFETY: info and params were checked non-null above.
    let info_ref = unsafe { &*info };
    let params_ref = unsafe { &*params };
    let path = unsafe { normalized_path_from_info(info) };
    log::info!("cloud_files: on_notify_delete for {path}");

    let handle = match TOKIO_HANDLE.get() {
        Some(h) => h,
        None => return,
    };
    let fs = match REMOTE_FS.get() {
        Some(f) => f,
        None => return,
    };

    let file_identity = extract_file_identity(info_ref);

    // SAFETY: This callback is NOTIFY_DELETE, so the Delete union variant
    // is the active field per the Cloud Filter driver contract.
    let is_directory = unsafe { params_ref.Anonymous.Delete.Flags }
        & CloudFilters::CF_CALLBACK_DELETE_FLAG_IS_DIRECTORY
        != CloudFilters::CF_CALLBACK_DELETE_FLAG_NONE;

    let result = handle.block_on(async {
        let (parent_path, name) = split_parent_name(&file_identity);
        let parent_ino = if parent_path.is_empty() {
            1u64
        } else {
            resolve_relative_path(fs, &parent_path).await?.ino
        };
        if is_directory {
            fs.rmdir(parent_ino, &name).await
        } else {
            fs.unlink(parent_ino, &name).await
        }
    });

    let completion_status = match result {
        Ok(()) => {
            log::info!("cloud_files: deleted remote {file_identity}");
            NTSTATUS(0)
        }
        Err(e) => {
            log::error!("cloud_files: remote delete failed for {file_identity}: {e}");
            NTSTATUS(0xC0000001u32 as i32) // STATUS_UNSUCCESSFUL
        }
    };

    ack_delete_response(info_ref, completion_status);
}

/// Callback invoked when a placeholder file or directory is about to be
/// renamed or moved locally. Propagates the rename to the remote filesystem
/// and responds with ACK_RENAME.
///
/// # Safety
///
/// Called by the Windows Cloud Filter driver. `info` and `params` must be
/// valid pointers to structures populated by the OS.
unsafe extern "system" fn on_notify_rename(
    info: *const CloudFilters::CF_CALLBACK_INFO,
    params: *const CloudFilters::CF_CALLBACK_PARAMETERS,
) {
    if info.is_null() || params.is_null() {
        log::error!("cloud_files: on_notify_rename called with null pointer");
        return;
    }

    // SAFETY: info and params were checked non-null above.
    let info_ref = unsafe { &*info };
    let params_ref = unsafe { &*params };
    let path = unsafe { normalized_path_from_info(info) };
    log::info!("cloud_files: on_notify_rename for {path}");

    let handle = match TOKIO_HANDLE.get() {
        Some(h) => h,
        None => return,
    };
    let fs = match REMOTE_FS.get() {
        Some(f) => f,
        None => return,
    };

    let source_identity = extract_file_identity(info_ref);

    // SAFETY: This callback is NOTIFY_RENAME, so the Rename union variant
    // is the active field per the Cloud Filter driver contract.
    let target_path_raw = unsafe { params_ref.Anonymous.Rename.TargetPath };
    let target_full = if target_path_raw.is_null() {
        log::error!("cloud_files: rename target path is null");
        ack_rename_response(info_ref, NTSTATUS(0xC0000001u32 as i32));
        return;
    } else {
        // SAFETY: TargetPath is populated by the OS as a valid wide string.
        unsafe {
            target_path_raw
                .to_string()
                .unwrap_or_else(|_| String::new())
        }
    };

    // Convert the full target path to a relative path within the mount.
    let mount_point = match MOUNT_POINT.get() {
        Some(p) => p,
        None => return,
    };
    let target_identity = relative_path(mount_point, Path::new(&target_full)).unwrap_or_default();

    if source_identity.is_empty() || target_identity.is_empty() {
        log::warn!(
            "cloud_files: rename with empty path — source={source_identity:?}, target={target_identity:?}"
        );
        ack_rename_response(info_ref, NTSTATUS(0));
        return;
    }

    let result = handle.block_on(async {
        let (src_parent, src_name) = split_parent_name(&source_identity);
        let (dst_parent, dst_name) = split_parent_name(&target_identity);

        let src_parent_ino = if src_parent.is_empty() {
            1u64
        } else {
            resolve_relative_path(fs, &src_parent).await?.ino
        };
        let dst_parent_ino = if dst_parent.is_empty() {
            1u64
        } else {
            resolve_relative_path(fs, &dst_parent).await?.ino
        };

        fs.rename(src_parent_ino, &src_name, dst_parent_ino, &dst_name)
            .await
    });

    let completion_status = match result {
        Ok(()) => {
            log::info!("cloud_files: renamed remote {source_identity} -> {target_identity}");
            NTSTATUS(0)
        }
        Err(e) => {
            log::error!(
                "cloud_files: remote rename failed {source_identity} -> {target_identity}: {e}"
            );
            NTSTATUS(0xC0000001u32 as i32)
        }
    };

    ack_rename_response(info_ref, completion_status);
}

/// Extracts the `FileIdentity` field from a callback info struct as a UTF-8
/// string.
///
/// Returns an empty string if the identity is not set (e.g., for the root
/// directory). Falls back to extracting a relative path from `NormalizedPath`
/// if `FileIdentity` is empty but the mount point is known.
fn extract_file_identity(info: &CloudFilters::CF_CALLBACK_INFO) -> String {
    if info.FileIdentityLength > 0 && !info.FileIdentity.is_null() {
        // SAFETY: FileIdentity is populated by the OS with the bytes we set
        // during placeholder creation. Length is given by FileIdentityLength.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                info.FileIdentity as *const u8,
                info.FileIdentityLength as usize,
            )
        };
        return String::from_utf8_lossy(bytes).into_owned();
    }

    // FileIdentity is empty — this may be the root directory or a path we
    // need to derive from NormalizedPath. Extract the NormalizedPath and
    // strip the mount point prefix to get a relative path.
    if let Some(mount_point) = MOUNT_POINT.get() {
        let normalized = info.NormalizedPath;
        if !normalized.is_null() {
            // SAFETY: NormalizedPath is populated by the OS as a valid wide string.
            if let Ok(full_path_str) = unsafe { normalized.to_string() } {
                let full_path = Path::new(&full_path_str);
                if let Some(rel) = relative_path(mount_point, full_path) {
                    // Normalize to forward slashes to match our FileIdentity
                    // convention (platform-agnostic, consistent with RemoteFs).
                    return rel.replace('\\', "/");
                }
            }
        }
    }

    String::new()
}

/// Fetches directory entries with file sizes in a single async operation.
///
/// Resolves the relative path to an inode, reads the directory, filters
/// out `.` and `..`, then fetches file sizes concurrently via `getattr`
/// using a `JoinSet` for parallel execution.
/// Returns a vec of `(name, file_type, size_in_bytes)` tuples.
async fn fetch_dir_entries(
    fs: &Arc<core::RemoteFs>,
    rel_path: &str,
) -> io::Result<Vec<(String, FileType, i64)>> {
    let ino = if rel_path.is_empty() {
        1u64
    } else {
        resolve_relative_path(fs, rel_path).await?.ino
    };

    let entries = fs.readdir(ino).await?;
    let filtered: Vec<_> = entries
        .into_iter()
        .filter(|e| e.name != "." && e.name != "..")
        .collect();

    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch file sizes concurrently using a JoinSet. Each task gets its
    // own Arc clone so the futures are Send + 'static.
    let mut join_set = tokio::task::JoinSet::new();
    for (idx, entry) in filtered.iter().enumerate() {
        let entry_ino = entry.ino;
        let is_dir = entry.file_type == FileType::Dir;
        let fs_clone = Arc::clone(fs);
        join_set.spawn(async move {
            let size = if is_dir {
                0i64
            } else {
                fs_clone
                    .getattr(entry_ino)
                    .await
                    .map(|attr| attr.size as i64)
                    .unwrap_or(0)
            };
            (idx, size)
        });
    }

    // Collect results indexed by position to preserve ordering.
    let mut sizes = vec![0i64; filtered.len()];
    while let Some(result) = join_set.join_next().await {
        if let Ok((idx, size)) = result {
            sizes[idx] = size;
        }
    }

    Ok(filtered
        .into_iter()
        .zip(sizes)
        .map(|(entry, size)| (entry.name, entry.file_type, size))
        .collect())
}

/// Resolves a backslash-separated relative path to a [`FileAttr`] by walking
/// each path component via [`RemoteFs::lookup`].
///
/// Returns the attributes (including inode) of the final component. For an
/// empty relative path, returns the root directory attributes.
///
/// # Errors
///
/// Returns an error if any component along the path cannot be found.
async fn resolve_relative_path(fs: &core::RemoteFs, rel_path: &str) -> io::Result<core::FileAttr> {
    if rel_path.is_empty() {
        return fs.getattr(1).await;
    }

    let typed = Utf8TypedPath::derive(rel_path);
    let mut current_ino = 1u64;
    let mut last_attr = None;

    for component in typed.components() {
        let name = component.as_str();
        if name.is_empty() {
            continue;
        }
        let attr = fs.lookup(current_ino, name).await?;
        current_ino = attr.ino;
        last_attr = Some(attr);
    }

    last_attr.ok_or_else(|| io::Error::other("empty path after splitting"))
}

/// Responds to a `FETCH_PLACEHOLDERS` callback by calling `CfExecute` with
/// `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS`.
///
/// The `flags` parameter controls population behavior. Pass
/// `CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION`
/// to mark the directory as fully populated and prevent the OS from
/// re-requesting placeholders.
fn transfer_placeholders_response(
    info: &CloudFilters::CF_CALLBACK_INFO,
    placeholders: &mut [CloudFilters::CF_PLACEHOLDER_CREATE_INFO],
    flags: CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAGS,
) {
    let op_info = CloudFilters::CF_OPERATION_INFO {
        StructSize: mem::size_of::<CloudFilters::CF_OPERATION_INFO>() as u32,
        Type: CloudFilters::CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: 0i64,
    };

    let mut params = CloudFilters::CF_OPERATION_PARAMETERS {
        ParamSize: mem::size_of::<CloudFilters::CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CloudFilters::CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: CloudFilters::CF_OPERATION_PARAMETERS_0_4 {
                Flags: flags,
                CompletionStatus: NTSTATUS(0),
                PlaceholderTotalCount: placeholders.len() as i64,
                PlaceholderArray: if placeholders.is_empty() {
                    std::ptr::null_mut()
                } else {
                    placeholders.as_mut_ptr()
                },
                PlaceholderCount: placeholders.len() as u32,
                EntriesProcessed: 0,
            },
        },
    };

    // SAFETY: op_info and params are valid structs on the stack.
    // ConnectionKey and TransferKey come from the callback info which
    // is valid for the duration of this callback invocation.
    let result = unsafe { CloudFilters::CfExecute(&op_info, &mut params) };

    // SAFETY: params is still valid after CfExecute returns. The
    // EntriesProcessed field is an output populated by the OS.
    let entries_processed = unsafe { params.Anonymous.TransferPlaceholders.EntriesProcessed };

    match result {
        Ok(()) => log::info!(
            "cloud_files: transferred {} placeholders, {} processed",
            placeholders.len(),
            entries_processed,
        ),
        Err(e) => log::error!("cloud_files: CfExecute TRANSFER_PLACEHOLDERS failed: {e}"),
    }
}

/// Responds to a `FETCH_DATA` callback by calling `CfExecute` with
/// `CF_OPERATION_TYPE_TRANSFER_DATA`.
///
/// When `success` is `false`, sends a `STATUS_UNSUCCESSFUL` completion
/// status to the OS so the application receives an appropriate error
/// instead of hanging on a timeout.
fn transfer_data_response(
    info: &CloudFilters::CF_CALLBACK_INFO,
    offset: i64,
    data: &[u8],
    success: bool,
) {
    let op_info = CloudFilters::CF_OPERATION_INFO {
        StructSize: mem::size_of::<CloudFilters::CF_OPERATION_INFO>() as u32,
        Type: CloudFilters::CF_OPERATION_TYPE_TRANSFER_DATA,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: 0i64,
    };

    let completion_status = if success {
        NTSTATUS(0)
    } else {
        NTSTATUS(0xC0000001u32 as i32)
    };

    let mut params = CloudFilters::CF_OPERATION_PARAMETERS {
        ParamSize: mem::size_of::<CloudFilters::CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CloudFilters::CF_OPERATION_PARAMETERS_0 {
            TransferData: CloudFilters::CF_OPERATION_PARAMETERS_0_0 {
                Flags: CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: completion_status,
                Buffer: if data.is_empty() {
                    std::ptr::null()
                } else {
                    data.as_ptr() as *const c_void
                },
                Offset: offset,
                Length: data.len() as i64,
            },
        },
    };

    // SAFETY: op_info and params are valid structs on the stack.
    // ConnectionKey and TransferKey come from the callback info which
    // is valid for the duration of this callback invocation.
    let result = unsafe { CloudFilters::CfExecute(&op_info, &mut params) };
    match result {
        Ok(()) => log::debug!(
            "cloud_files: transferred {} bytes at offset {offset}",
            data.len()
        ),
        Err(e) => log::error!("cloud_files: CfExecute TRANSFER_DATA failed: {e}"),
    }
}

/// Responds to a `NOTIFY_DELETE` callback by calling `CfExecute` with
/// `CF_OPERATION_TYPE_ACK_DELETE`.
fn ack_delete_response(info: &CloudFilters::CF_CALLBACK_INFO, status: NTSTATUS) {
    let op_info = CloudFilters::CF_OPERATION_INFO {
        StructSize: mem::size_of::<CloudFilters::CF_OPERATION_INFO>() as u32,
        Type: CloudFilters::CF_OPERATION_TYPE_ACK_DELETE,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: 0i64,
    };

    let mut params = CloudFilters::CF_OPERATION_PARAMETERS {
        ParamSize: mem::size_of::<CloudFilters::CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CloudFilters::CF_OPERATION_PARAMETERS_0 {
            AckDelete: CloudFilters::CF_OPERATION_PARAMETERS_0_7 {
                Flags: CloudFilters::CF_OPERATION_ACK_DELETE_FLAG_NONE,
                CompletionStatus: status,
            },
        },
    };

    // SAFETY: op_info and params are valid structs on the stack.
    let result = unsafe { CloudFilters::CfExecute(&op_info, &mut params) };
    if let Err(e) = result {
        log::error!("cloud_files: CfExecute ACK_DELETE failed: {e}");
    }
}

/// Responds to a `NOTIFY_RENAME` callback by calling `CfExecute` with
/// `CF_OPERATION_TYPE_ACK_RENAME`.
fn ack_rename_response(info: &CloudFilters::CF_CALLBACK_INFO, status: NTSTATUS) {
    let op_info = CloudFilters::CF_OPERATION_INFO {
        StructSize: mem::size_of::<CloudFilters::CF_OPERATION_INFO>() as u32,
        Type: CloudFilters::CF_OPERATION_TYPE_ACK_RENAME,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: 0i64,
    };

    let mut params = CloudFilters::CF_OPERATION_PARAMETERS {
        ParamSize: mem::size_of::<CloudFilters::CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CloudFilters::CF_OPERATION_PARAMETERS_0 {
            AckRename: CloudFilters::CF_OPERATION_PARAMETERS_0_6 {
                Flags: CloudFilters::CF_OPERATION_ACK_RENAME_FLAG_NONE,
                CompletionStatus: status,
            },
        },
    };

    // SAFETY: op_info and params are valid structs on the stack.
    let result = unsafe { CloudFilters::CfExecute(&op_info, &mut params) };
    if let Err(e) = result {
        log::error!("cloud_files: CfExecute ACK_RENAME failed: {e}");
    }
}

/// Splits a relative path into `(parent_path, file_name)`.
///
/// Uses `Utf8TypedPath::derive()` to handle both `/` and `\` separators,
/// so it works regardless of whether the identity uses Unix or Windows
/// path conventions.
///
/// For a single component like `"file.txt"`, returns `("", "file.txt")`.
/// For `"subdir/file.txt"`, returns `("subdir", "file.txt")`.
fn split_parent_name(path: &str) -> (String, String) {
    let typed = Utf8TypedPath::derive(path);
    let name = typed.file_name().unwrap_or(path).to_string();
    let parent = typed
        .parent()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    (parent, name)
}

/// Converts an absolute path from the Cloud Filter API into a relative path
/// within the remote filesystem.
///
/// Returns `None` if `full_path` is the mount root itself (no relative
/// component), or if `full_path` is not under `mount_point`.
fn relative_path(mount_point: &Path, full_path: &Path) -> Option<String> {
    full_path.strip_prefix(mount_point).ok().and_then(|rel| {
        let s = rel.to_string_lossy();
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    })
}

/// Builds a deterministic sync root ID string for distant.
///
/// The format `distant!default` is used for Phase 1. Phase 5 will add
/// per-user and per-machine uniqueness.
/// Builds a sync root ID string incorporating the mount point path.
///
/// Each mount gets a unique ID based on the path hash so multiple mounts
/// on the same machine don't collide. The format is `distant!<hash>` where
/// the hash is derived from the mount point's canonical path.
fn build_sync_root_id(mount_point: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    mount_point.hash(&mut hasher);
    let hash = hasher.finish();
    format!("distant!{hash:016x}")
}
