use std::any::TypeId;
use std::io;
use std::path::PathBuf;

use async_trait::async_trait;
use distant_core_protocol::*;

mod ctx;
pub use ctx::*;

/// Full API that represents a distant-compatible server.
#[async_trait]
pub trait Api {
    type FileSystem: FileSystemApi;
    type Process: ProcessApi;
    type Search: SearchApi;
    type SystemInfo: SystemInfoApi;
    type Version: VersionApi;
    type Watch: WatchApi;

    /// Returns true if [`FileSystemApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_file_system_api_supported() -> bool {
        TypeId::of::<Self::FileSystem>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`ProcessApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_process_api_supported() -> bool {
        TypeId::of::<Self::Process>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`SearchApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_search_api_supported() -> bool {
        TypeId::of::<Self::Search>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`SystemInfoApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_system_info_api_supported() -> bool {
        TypeId::of::<Self::SystemInfo>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`VersionApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_version_api_supported() -> bool {
        TypeId::of::<Self::Version>() != TypeId::of::<Unsupported>()
    }

    /// Returns true if [`WatchApi`] is supported. This is checked by ensuring that the
    /// implementation of the associated trait is not [`Unsupported`].
    fn is_watch_api_supported() -> bool {
        TypeId::of::<Self::Watch>() != TypeId::of::<Unsupported>()
    }
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

pub use unsupported::Unsupported;

mod unsupported {
    use super::*;

    #[inline]
    fn unsupported<T>(label: &str) -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{label} is unsupported"),
        ))
    }

    /// Generic struct that implements all APIs as unsupported.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Unsupported;

    #[async_trait]
    impl Api for Unsupported {
        type FileSystem = Self;
        type Process = Self;
        type Search = Self;
        type SystemInfo = Self;
        type Version = Self;
        type Watch = Self;
    }

    #[async_trait]
    impl FileSystemApi for Unsupported {
        async fn read_file(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<Vec<u8>> {
            unsupported("read_file")
        }

        async fn read_file_text(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<String> {
            unsupported("read_file_text")
        }

        async fn write_file(&self, ctx: BoxedCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
            unsupported("write_file")
        }

        async fn write_file_text(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            data: String,
        ) -> io::Result<()> {
            unsupported("write_file_text")
        }

        async fn append_file(&self, ctx: BoxedCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
            unsupported("append_file")
        }

        async fn append_file_text(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            data: String,
        ) -> io::Result<()> {
            unsupported("append_file_text")
        }

        async fn read_dir(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            depth: usize,
            absolute: bool,
            canonicalize: bool,
            include_root: bool,
        ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
            unsupported("read_dir")
        }

        async fn create_dir(&self, ctx: BoxedCtx, path: PathBuf, all: bool) -> io::Result<()> {
            unsupported("create_dir")
        }

        async fn copy(&self, ctx: BoxedCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
            unsupported("copy")
        }

        async fn remove(&self, ctx: BoxedCtx, path: PathBuf, force: bool) -> io::Result<()> {
            unsupported("remove")
        }

        async fn rename(&self, ctx: BoxedCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
            unsupported("rename")
        }

        async fn exists(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<bool> {
            unsupported("exists")
        }

        async fn metadata(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            canonicalize: bool,
            resolve_file_type: bool,
        ) -> io::Result<Metadata> {
            unsupported("metadata")
        }

        async fn set_permissions(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            permissions: Permissions,
            options: SetPermissionsOptions,
        ) -> io::Result<()> {
            unsupported("set_permissions")
        }
    }

    #[async_trait]
    impl ProcessApi for Unsupported {
        async fn proc_spawn(
            &self,
            ctx: BoxedCtx,
            cmd: String,
            environment: Environment,
            current_dir: Option<PathBuf>,
            pty: Option<PtySize>,
        ) -> io::Result<ProcessId> {
            unsupported("proc_spawn")
        }

        async fn proc_kill(&self, ctx: BoxedCtx, id: ProcessId) -> io::Result<()> {
            unsupported("proc_kill")
        }

        async fn proc_stdin(&self, ctx: BoxedCtx, id: ProcessId, data: Vec<u8>) -> io::Result<()> {
            unsupported("proc_stdin")
        }

        async fn proc_resize_pty(
            &self,
            ctx: BoxedCtx,
            id: ProcessId,
            size: PtySize,
        ) -> io::Result<()> {
            unsupported("proc_resize_pty")
        }
    }

    #[async_trait]
    impl SearchApi for Unsupported {
        async fn search(&self, ctx: BoxedCtx, query: SearchQuery) -> io::Result<SearchId> {
            unsupported("search")
        }

        async fn cancel_search(&self, ctx: BoxedCtx, id: SearchId) -> io::Result<()> {
            unsupported("cancel_search")
        }
    }

    #[async_trait]
    impl SystemInfoApi for Unsupported {
        async fn system_info(&self, ctx: BoxedCtx) -> io::Result<SystemInfo> {
            unsupported("system_info")
        }
    }

    #[async_trait]
    impl VersionApi for Unsupported {
        async fn version(&self, ctx: BoxedCtx) -> io::Result<Version> {
            unsupported("version")
        }
    }

    #[async_trait]
    impl WatchApi for Unsupported {
        async fn watch(
            &self,
            ctx: BoxedCtx,
            path: PathBuf,
            recursive: bool,
            only: Vec<ChangeKind>,
            except: Vec<ChangeKind>,
        ) -> io::Result<()> {
            unsupported("watch")
        }

        async fn unwatch(&self, ctx: BoxedCtx, path: PathBuf) -> io::Result<()> {
            unsupported("unwatch")
        }
    }
}
