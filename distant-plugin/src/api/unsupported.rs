use async_trait::async_trait;

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

    fn file_system(&self) -> &Self::FileSystem {
        self
    }

    fn process(&self) -> &Self::Process {
        self
    }

    fn search(&self) -> &Self::Search {
        self
    }

    fn system_info(&self) -> &Self::SystemInfo {
        self
    }

    fn version(&self) -> &Self::Version {
        self
    }

    fn watch(&self) -> &Self::Watch {
        self
    }
}

#[async_trait]
impl FileSystemApi for Unsupported {
    async fn read_file(&self, _ctx: BoxedCtx, _path: PathBuf) -> io::Result<Vec<u8>> {
        unsupported("read_file")
    }

    async fn read_file_text(&self, _ctx: BoxedCtx, _path: PathBuf) -> io::Result<String> {
        unsupported("read_file_text")
    }

    async fn write_file(&self, _ctx: BoxedCtx, _path: PathBuf, _data: Vec<u8>) -> io::Result<()> {
        unsupported("write_file")
    }

    async fn write_file_text(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _data: String,
    ) -> io::Result<()> {
        unsupported("write_file_text")
    }

    async fn append_file(&self, _ctx: BoxedCtx, _path: PathBuf, _data: Vec<u8>) -> io::Result<()> {
        unsupported("append_file")
    }

    async fn append_file_text(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _data: String,
    ) -> io::Result<()> {
        unsupported("append_file_text")
    }

    async fn read_dir(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _depth: usize,
        _absolute: bool,
        _canonicalize: bool,
        _include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
        unsupported("read_dir")
    }

    async fn create_dir(&self, _ctx: BoxedCtx, _path: PathBuf, _all: bool) -> io::Result<()> {
        unsupported("create_dir")
    }

    async fn copy(&self, _ctx: BoxedCtx, _src: PathBuf, _dst: PathBuf) -> io::Result<()> {
        unsupported("copy")
    }

    async fn remove(&self, _ctx: BoxedCtx, _path: PathBuf, _force: bool) -> io::Result<()> {
        unsupported("remove")
    }

    async fn rename(&self, _ctx: BoxedCtx, _src: PathBuf, _dst: PathBuf) -> io::Result<()> {
        unsupported("rename")
    }

    async fn exists(&self, _ctx: BoxedCtx, _path: PathBuf) -> io::Result<bool> {
        unsupported("exists")
    }

    async fn metadata(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _canonicalize: bool,
        _resolve_file_type: bool,
    ) -> io::Result<Metadata> {
        unsupported("metadata")
    }

    async fn set_permissions(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _permissions: Permissions,
        _options: SetPermissionsOptions,
    ) -> io::Result<()> {
        unsupported("set_permissions")
    }
}

#[async_trait]
impl ProcessApi for Unsupported {
    async fn proc_spawn(
        &self,
        _ctx: BoxedCtx,
        _cmd: String,
        _environment: Environment,
        _current_dir: Option<PathBuf>,
        _pty: Option<PtySize>,
    ) -> io::Result<ProcessId> {
        unsupported("proc_spawn")
    }

    async fn proc_kill(&self, _ctx: BoxedCtx, _id: ProcessId) -> io::Result<()> {
        unsupported("proc_kill")
    }

    async fn proc_stdin(&self, _ctx: BoxedCtx, _id: ProcessId, _data: Vec<u8>) -> io::Result<()> {
        unsupported("proc_stdin")
    }

    async fn proc_resize_pty(
        &self,
        _ctx: BoxedCtx,
        _id: ProcessId,
        _size: PtySize,
    ) -> io::Result<()> {
        unsupported("proc_resize_pty")
    }
}

#[async_trait]
impl SearchApi for Unsupported {
    async fn search(&self, _ctx: BoxedCtx, _query: SearchQuery) -> io::Result<SearchId> {
        unsupported("search")
    }

    async fn cancel_search(&self, _ctx: BoxedCtx, _id: SearchId) -> io::Result<()> {
        unsupported("cancel_search")
    }
}

#[async_trait]
impl SystemInfoApi for Unsupported {
    async fn system_info(&self, _ctx: BoxedCtx) -> io::Result<SystemInfo> {
        unsupported("system_info")
    }
}

#[async_trait]
impl VersionApi for Unsupported {
    async fn version(&self, _ctx: BoxedCtx) -> io::Result<Version> {
        unsupported("version")
    }
}

#[async_trait]
impl WatchApi for Unsupported {
    async fn watch(
        &self,
        _ctx: BoxedCtx,
        _path: PathBuf,
        _recursive: bool,
        _only: Vec<ChangeKind>,
        _except: Vec<ChangeKind>,
    ) -> io::Result<()> {
        unsupported("watch")
    }

    async fn unwatch(&self, _ctx: BoxedCtx, _path: PathBuf) -> io::Result<()> {
        unsupported("unwatch")
    }
}
