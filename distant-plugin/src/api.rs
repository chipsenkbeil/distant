use std::io;
use std::path::PathBuf;

use async_trait::async_trait;
use distant_core_protocol::*;

mod checker;
mod ctx;
mod unsupported;

pub use checker::*;
pub use ctx::*;
pub use unsupported::*;

/// Full API that represents a distant-compatible server.
#[async_trait]
pub trait Api {
    /// Specific implementation of [`FileSystemApi`] associated with this [`Api`].
    type FileSystem: FileSystemApi;

    /// Specific implementation of [`ProcessApi`] associated with this [`Api`].
    type Process: ProcessApi;

    /// Specific implementation of [`SearchApi`] associated with this [`Api`].
    type Search: SearchApi;

    /// Specific implementation of [`SystemInfoApi`] associated with this [`Api`].
    type SystemInfo: SystemInfoApi;

    /// Specific implementation of [`VersionApi`] associated with this [`Api`].
    type Version: VersionApi;

    /// Specific implementation of [`WatchApi`] associated with this [`Api`].
    type Watch: WatchApi;

    /// Returns a reference to the [`FileSystemApi`] implementation tied to this [`Api`].
    fn file_system(&self) -> &Self::FileSystem;

    /// Returns a reference to the [`ProcessApi`] implementation tied to this [`Api`].
    fn process(&self) -> &Self::Process;

    /// Returns a reference to the [`SearchApi`] implementation tied to this [`Api`].
    fn search(&self) -> &Self::Search;

    /// Returns a reference to the [`SystemInfoApi`] implementation tied to this [`Api`].
    fn system_info(&self) -> &Self::SystemInfo;

    /// Returns a reference to the [`VersionApi`] implementation tied to this [`Api`].
    fn version(&self) -> &Self::Version;

    /// Returns a reference to the [`WatchApi`] implementation tied to this [`Api`].
    fn watch(&self) -> &Self::Watch;
}

/// API supporting filesystem operations.
#[async_trait]
pub trait FileSystemApi {
    /// Reads bytes from a file.
    ///
    /// * `path` - the path to the file
    async fn read_file(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<Vec<u8>>;

    /// Reads bytes from a file as text.
    ///
    /// * `path` - the path to the file
    async fn read_file_text(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<String>;

    /// Writes bytes to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    async fn write_file(&self, ctx: BoxedCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()>;

    /// Writes text to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    async fn write_file_text(&self, ctx: BoxedCtx, path: PathBuf, data: String) -> io::Result<()>;

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    async fn append_file(&self, ctx: BoxedCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()>;

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    async fn append_file_text(&self, ctx: BoxedCtx, path: PathBuf, data: String) -> io::Result<()>;

    /// Reads entries from a directory.
    ///
    /// * `path` - the path to the directory
    /// * `depth` - how far to traverse the directory, 0 being unlimited
    /// * `absolute` - if true, will return absolute paths instead of relative paths
    /// * `canonicalize` - if true, will canonicalize entry paths before returned
    /// * `include_root` - if true, will include the directory specified in the entries
    async fn read_dir(
        &self,
        ctx: BoxedCtx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)>;

    /// Creates a directory.
    ///
    /// * `path` - the path to the directory
    /// * `all` - if true, will create all missing parent components
    async fn create_dir(&self, ctx: BoxedCtx, path: PathBuf, all: bool) -> io::Result<()>;

    /// Copies some file or directory.
    ///
    /// * `src` - the path to the file or directory to copy
    /// * `dst` - the path where the copy will be placed
    async fn copy(&self, ctx: BoxedCtx, src: PathBuf, dst: PathBuf) -> io::Result<()>;

    /// Removes some file or directory.
    ///
    /// * `path` - the path to a file or directory
    /// * `force` - if true, will remove non-empty directories
    async fn remove(&self, ctx: BoxedCtx, path: PathBuf, force: bool) -> io::Result<()>;

    /// Renames some file or directory.
    ///
    /// * `src` - the path to the file or directory to rename
    /// * `dst` - the new name for the file or directory
    async fn rename(&self, ctx: BoxedCtx, src: PathBuf, dst: PathBuf) -> io::Result<()>;

    /// Checks if the specified path exists.
    ///
    /// * `path` - the path to the file or directory
    async fn exists(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<bool>;

    /// Reads metadata for a file or directory.
    ///
    /// * `path` - the path to the file or directory
    /// * `canonicalize` - if true, will include a canonicalized path in the metadata
    /// * `resolve_file_type` - if true, will resolve symlinks to underlying type (file or dir)
    async fn metadata(
        &self,
        ctx: BoxedCtx,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata>;

    /// Sets permissions for a file, directory, or symlink.
    ///
    /// * `path` - the path to the file, directory, or symlink
    /// * `resolve_symlink` - if true, will resolve the path to the underlying file/directory
    /// * `permissions` - the new permissions to apply
    async fn set_permissions(
        &self,
        ctx: BoxedCtx,
        path: PathBuf,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> io::Result<()>;
}

/// API supporting process creation and manipulation.
#[async_trait]
pub trait ProcessApi {
    /// Spawns a new process, returning its id.
    ///
    /// * `cmd` - the full command to run as a new process (including arguments)
    /// * `environment` - the environment variables to associate with the process
    /// * `current_dir` - the alternative current directory to use with the process
    /// * `pty` - if provided, will run the process within a PTY of the given size
    async fn proc_spawn(
        &self,
        ctx: BoxedCtx,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> io::Result<ProcessId>;

    /// Kills a running process by its id.
    ///
    /// * `id` - the unique id of the process
    async fn proc_kill(&self, ctx: BoxedCtx, id: ProcessId) -> io::Result<()>;

    /// Sends data to the stdin of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `data` - the bytes to send to stdin
    async fn proc_stdin(&self, ctx: BoxedCtx, id: ProcessId, data: Vec<u8>) -> io::Result<()>;

    /// Resizes the PTY of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `size` - the new size of the pty
    async fn proc_resize_pty(&self, ctx: BoxedCtx, id: ProcessId, size: PtySize) -> io::Result<()>;
}

/// API supporting searching through the remote system.
#[async_trait]
pub trait SearchApi {
    /// Searches files for matches based on a query.
    ///
    /// * `query` - the specific query to perform
    async fn search(&self, ctx: BoxedCtx, query: SearchQuery) -> io::Result<SearchId>;

    /// Cancels an actively-ongoing search.
    ///
    /// * `id` - the id of the search to cancel
    async fn cancel_search(&self, ctx: BoxedCtx, id: SearchId) -> io::Result<()>;
}

/// API supporting retrieval of information about the remote system.
#[async_trait]
pub trait SystemInfoApi {
    /// Retrieves information about the system.
    async fn system_info(&self, ctx: BoxedCtx) -> io::Result<SystemInfo>;
}

/// API supporting retrieval of the server's version.
#[async_trait]
pub trait VersionApi {
    /// Retrieves information about the server's capabilities.
    async fn version(&self, ctx: BoxedCtx) -> io::Result<Version>;
}

/// API supporting watching of changes to the remote filesystem.
#[async_trait]
pub trait WatchApi {
    /// Watches a file or directory for changes.
    ///
    /// * `path` - the path to the file or directory
    /// * `recursive` - if true, will watch for changes within subdirectories and beyond
    /// * `only` - if non-empty, will limit reported changes to those included in this list
    /// * `except` - if non-empty, will limit reported changes to those not included in this list
    async fn watch(
        &self,
        ctx: BoxedCtx,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> io::Result<()>;

    /// Removes a file or directory from being watched.
    ///
    /// * `path` - the path to the file or directory
    async fn unwatch(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<()>;
}
