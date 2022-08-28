use crate::{
    data::{
        Capabilities, ChangeKind, DirEntry, Environment, Error, Metadata, ProcessId, PtySize,
        SystemInfo,
    },
    ConnectionId, DistantMsg, DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use distant_net::{Reply, Server, ServerConfig, ServerCtx};
use log::*;
use std::{io, path::PathBuf, sync::Arc};

mod local;
pub use local::LocalDistantApi;

mod reply;
use reply::DistantSingleReply;

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub struct DistantCtx<T> {
    pub connection_id: ConnectionId,
    pub reply: Box<dyn Reply<Data = DistantResponseData>>,
    pub local_data: Arc<T>,
}

/// Represents a server that leverages an API compliant with `distant`
pub struct DistantApiServer<T, D>
where
    T: DistantApi<LocalData = D>,
{
    api: T,
}

impl<T, D> DistantApiServer<T, D>
where
    T: DistantApi<LocalData = D>,
{
    pub fn new(api: T) -> Self {
        Self { api }
    }
}

impl DistantApiServer<LocalDistantApi, <LocalDistantApi as DistantApi>::LocalData> {
    /// Creates a new server using the [`LocalDistantApi`] implementation
    pub fn local(config: ServerConfig) -> io::Result<Self> {
        Ok(Self {
            api: LocalDistantApi::initialize(config)?,
        })
    }
}

#[inline]
fn unsupported<T>(label: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{} is unsupported", label),
    ))
}

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
#[async_trait]
pub trait DistantApi {
    type LocalData: Send + Sync;

    /// Returns config associated with API server
    fn config(&self) -> ServerConfig {
        ServerConfig::default()
    }

    /// Invoked whenever a new connection is established, providing a mutable reference to the
    /// newly-created local data. This is a way to support modifying local data before it is used.
    #[allow(unused_variables)]
    async fn on_accept(&self, local_data: &mut Self::LocalData) {}

    /// Retrieves information about the server's capabilities.
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn capabilities(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<Capabilities> {
        unsupported("capabilities")
    }

    /// Reads bytes from a file.
    ///
    /// * `path` - the path to the file
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn read_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<Vec<u8>> {
        unsupported("read_file")
    }

    /// Reads bytes from a file as text.
    ///
    /// * `path` - the path to the file
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn read_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<String> {
        unsupported("read_file_text")
    }

    /// Writes bytes to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn write_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()> {
        unsupported("write_file")
    }

    /// Writes text to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn write_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        unsupported("write_file_text")
    }

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn append_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()> {
        unsupported("append_file")
    }

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn append_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        unsupported("append_file_text")
    }

    /// Reads entries from a directory.
    ///
    /// * `path` - the path to the directory
    /// * `depth` - how far to traverse the directory, 0 being unlimited
    /// * `absolute` - if true, will return absolute paths instead of relative paths
    /// * `canonicalize` - if true, will canonicalize entry paths before returned
    /// * `include_root` - if true, will include the directory specified in the entries
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn read_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
        unsupported("read_dir")
    }

    /// Creates a directory.
    ///
    /// * `path` - the path to the directory
    /// * `all` - if true, will create all missing parent components
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn create_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        all: bool,
    ) -> io::Result<()> {
        unsupported("create_dir")
    }

    /// Copies some file or directory.
    ///
    /// * `src` - the path to the file or directory to copy
    /// * `dst` - the path where the copy will be placed
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn copy(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()> {
        unsupported("copy")
    }

    /// Removes some file or directory.
    ///
    /// * `path` - the path to a file or directory
    /// * `force` - if true, will remove non-empty directories
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn remove(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        force: bool,
    ) -> io::Result<()> {
        unsupported("remove")
    }

    /// Renames some file or directory.
    ///
    /// * `src` - the path to the file or directory to rename
    /// * `dst` - the new name for the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn rename(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()> {
        unsupported("rename")
    }

    /// Watches a file or directory for changes.
    ///
    /// * `path` - the path to the file or directory
    /// * `recursive` - if true, will watch for changes within subdirectories and beyond
    /// * `only` - if non-empty, will limit reported changes to those included in this list
    /// * `except` - if non-empty, will limit reported changes to those not included in this list
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn watch(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> io::Result<()> {
        unsupported("watch")
    }

    /// Removes a file or directory from being watched.
    ///
    /// * `path` - the path to the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn unwatch(&self, ctx: DistantCtx<Self::LocalData>, path: PathBuf) -> io::Result<()> {
        unsupported("unwatch")
    }

    /// Checks if the specified path exists.
    ///
    /// * `path` - the path to the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn exists(&self, ctx: DistantCtx<Self::LocalData>, path: PathBuf) -> io::Result<bool> {
        unsupported("exists")
    }

    /// Reads metadata for a file or directory.
    ///
    /// * `path` - the path to the file or directory
    /// * `canonicalize` - if true, will include a canonicalized path in the metadata
    /// * `resolve_file_type` - if true, will resolve symlinks to underlying type (file or dir)
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn metadata(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata> {
        unsupported("metadata")
    }

    /// Spawns a new process, returning its id.
    ///
    /// * `cmd` - the full command to run as a new process (including arguments)
    /// * `environment` - the environment variables to associate with the process
    /// * `current_dir` - the alternative current directory to use with the process
    /// * `persist` - if true, the process will continue running even after the connection that
    ///               spawned the process has terminated
    /// * `pty` - if provided, will run the process within a PTY of the given size
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn proc_spawn(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> io::Result<ProcessId> {
        unsupported("proc_spawn")
    }

    /// Kills a running process by its id.
    ///
    /// * `id` - the unique id of the process
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn proc_kill(&self, ctx: DistantCtx<Self::LocalData>, id: ProcessId) -> io::Result<()> {
        unsupported("proc_kill")
    }

    /// Sends data to the stdin of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `data` - the bytes to send to stdin
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn proc_stdin(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: ProcessId,
        data: Vec<u8>,
    ) -> io::Result<()> {
        unsupported("proc_stdin")
    }

    /// Resizes the PTY of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `size` - the new size of the pty
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn proc_resize_pty(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: ProcessId,
        size: PtySize,
    ) -> io::Result<()> {
        unsupported("proc_resize_pty")
    }

    /// Retrieves information about the system.
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    async fn system_info(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<SystemInfo> {
        unsupported("system_info")
    }
}

#[async_trait]
impl<T, D> Server for DistantApiServer<T, D>
where
    T: DistantApi<LocalData = D> + Send + Sync,
    D: Send + Sync,
{
    type Request = DistantMsg<DistantRequestData>;
    type Response = DistantMsg<DistantResponseData>;
    type LocalData = D;

    /// Overridden to leverage [`DistantApi`] implementation of `config`
    fn config(&self) -> ServerConfig {
        T::config(&self.api)
    }

    /// Overridden to leverage [`DistantApi`] implementation of `on_accept`
    async fn on_accept(&self, local_data: &mut Self::LocalData) {
        T::on_accept(&self.api, local_data).await
    }

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            local_data,
        } = ctx;

        // Convert our reply to a queued reply so we can ensure that the result
        // of an API function is sent back before anything else
        let reply = reply.queue();

        // Process single vs batch requests
        let response = match request.payload {
            DistantMsg::Single(data) => {
                let ctx = DistantCtx {
                    connection_id,
                    reply: Box::new(DistantSingleReply::from(reply.clone_reply())),
                    local_data,
                };

                let data = handle_request(self, ctx, data).await;

                // Report outgoing errors in our debug logs
                if let DistantResponseData::Error(x) = &data {
                    debug!("[Conn {}] {}", connection_id, x);
                }

                DistantMsg::Single(data)
            }
            DistantMsg::Batch(list) => {
                let mut out = Vec::new();

                for data in list {
                    let ctx = DistantCtx {
                        connection_id,
                        reply: Box::new(DistantSingleReply::from(reply.clone_reply())),
                        local_data: Arc::clone(&local_data),
                    };

                    // TODO: This does not run in parallel, meaning that the next item in the
                    //       batch will not be queued until the previous item completes! This
                    //       would be useful if we wanted to chain requests where the previous
                    //       request feeds into the current request, but not if we just want
                    //       to run everything together. So we should instead rewrite this
                    //       to spawn a task per request and then await completion of all tasks
                    let data = handle_request(self, ctx, data).await;

                    // Report outgoing errors in our debug logs
                    if let DistantResponseData::Error(x) = &data {
                        debug!("[Conn {}] {}", connection_id, x);
                    }

                    out.push(data);
                }

                DistantMsg::Batch(out)
            }
        };

        // Queue up our result to go before ANY of the other messages that might be sent.
        // This is important to avoid situations such as when a process is started, but before
        // the confirmation can be sent some stdout or stderr is captured and sent first.
        if let Err(x) = reply.send_before(response).await {
            error!("[Conn {}] Failed to send response: {}", connection_id, x);
        }

        // Flush out all of our replies thus far and toggle to no longer hold submissions
        if let Err(x) = reply.flush(false).await {
            error!(
                "[Conn {}] Failed to flush response queue: {}",
                connection_id, x
            );
        }
    }
}

/// Processes an incoming request
async fn handle_request<T, D>(
    server: &DistantApiServer<T, D>,
    ctx: DistantCtx<D>,
    request: DistantRequestData,
) -> DistantResponseData
where
    T: DistantApi<LocalData = D> + Send + Sync,
    D: Send + Sync,
{
    match request {
        DistantRequestData::Capabilities {} => server
            .api
            .capabilities(ctx)
            .await
            .map(|supported| DistantResponseData::Capabilities { supported })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileRead { path } => server
            .api
            .read_file(ctx, path)
            .await
            .map(|data| DistantResponseData::Blob { data })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileReadText { path } => server
            .api
            .read_file_text(ctx, path)
            .await
            .map(|data| DistantResponseData::Text { data })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileWrite { path, data } => server
            .api
            .write_file(ctx, path, data)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileWriteText { path, text } => server
            .api
            .write_file_text(ctx, path, text)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileAppend { path, data } => server
            .api
            .append_file(ctx, path, data)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::FileAppendText { path, text } => server
            .api
            .append_file_text(ctx, path, text)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::DirRead {
            path,
            depth,
            absolute,
            canonicalize,
            include_root,
        } => server
            .api
            .read_dir(ctx, path, depth, absolute, canonicalize, include_root)
            .await
            .map(|(entries, errors)| DistantResponseData::DirEntries {
                entries,
                errors: errors.into_iter().map(Error::from).collect(),
            })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::DirCreate { path, all } => server
            .api
            .create_dir(ctx, path, all)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Remove { path, force } => server
            .api
            .remove(ctx, path, force)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Copy { src, dst } => server
            .api
            .copy(ctx, src, dst)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Rename { src, dst } => server
            .api
            .rename(ctx, src, dst)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Watch {
            path,
            recursive,
            only,
            except,
        } => server
            .api
            .watch(ctx, path, recursive, only, except)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Unwatch { path } => server
            .api
            .unwatch(ctx, path)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Exists { path } => server
            .api
            .exists(ctx, path)
            .await
            .map(|value| DistantResponseData::Exists { value })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::Metadata {
            path,
            canonicalize,
            resolve_file_type,
        } => server
            .api
            .metadata(ctx, path, canonicalize, resolve_file_type)
            .await
            .map(DistantResponseData::Metadata)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::ProcSpawn {
            cmd,
            environment,
            current_dir,
            persist,
            pty,
        } => server
            .api
            .proc_spawn(ctx, cmd.into(), environment, current_dir, persist, pty)
            .await
            .map(|id| DistantResponseData::ProcSpawned { id })
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::ProcKill { id } => server
            .api
            .proc_kill(ctx, id)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::ProcStdin { id, data } => server
            .api
            .proc_stdin(ctx, id, data)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::ProcResizePty { id, size } => server
            .api
            .proc_resize_pty(ctx, id, size)
            .await
            .map(|_| DistantResponseData::Ok)
            .unwrap_or_else(DistantResponseData::from),
        DistantRequestData::SystemInfo {} => server
            .api
            .system_info(ctx)
            .await
            .map(DistantResponseData::SystemInfo)
            .unwrap_or_else(DistantResponseData::from),
    }
}
