use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use crate::net::common::ConnectionId;
use crate::net::server::{Reply, RequestCtx, ServerHandler};
use log::*;

use crate::protocol::{
    self, ChangeKind, DirEntry, Environment, Error, Metadata, Permissions, ProcessId, PtySize,
    SearchId, SearchQuery, SetPermissionsOptions, SystemInfo, Version,
};

mod reply;
use reply::SingleReply;

/// Represents the context provided to the [`Api`] for incoming requests
pub struct Ctx {
    pub connection_id: ConnectionId,
    pub reply: Box<dyn Reply<Data = protocol::Response>>,
}

/// Represents a [`ServerHandler`] that leverages an API compliant with `distant`
pub struct ApiServerHandler<T>
where
    T: Api,
{
    api: Arc<T>,
}

impl<T> ApiServerHandler<T>
where
    T: Api,
{
    pub fn new(api: T) -> Self {
        Self { api: Arc::new(api) }
    }
}

#[inline]
fn unsupported<T>(label: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{label} is unsupported"),
    ))
}

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
pub trait Api {
    /// Invoked whenever a new connection is established.
    #[allow(unused_variables)]
    fn on_connect(&self, id: ConnectionId) -> impl Future<Output = io::Result<()>> + Send {
        async { Ok(()) }
    }

    /// Invoked whenever an existing connection is dropped.
    #[allow(unused_variables)]
    fn on_disconnect(&self, id: ConnectionId) -> impl Future<Output = io::Result<()>> + Send {
        async { Ok(()) }
    }

    /// Retrieves information about the server's capabilities.
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn version(&self, ctx: Ctx) -> impl Future<Output = io::Result<Version>> + Send {
        async { unsupported("version") }
    }

    /// Reads bytes from a file.
    ///
    /// * `path` - the path to the file
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn read_file(
        &self,
        ctx: Ctx,
        path: PathBuf,
    ) -> impl Future<Output = io::Result<Vec<u8>>> + Send {
        async { unsupported("read_file") }
    }

    /// Reads bytes from a file as text.
    ///
    /// * `path` - the path to the file
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn read_file_text(
        &self,
        ctx: Ctx,
        path: PathBuf,
    ) -> impl Future<Output = io::Result<String>> + Send {
        async { unsupported("read_file_text") }
    }

    /// Writes bytes to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn write_file(
        &self,
        ctx: Ctx,
        path: PathBuf,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("write_file") }
    }

    /// Writes text to a file, overwriting the file if it exists.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to write
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn write_file_text(
        &self,
        ctx: Ctx,
        path: PathBuf,
        data: String,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("write_file_text") }
    }

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn append_file(
        &self,
        ctx: Ctx,
        path: PathBuf,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("append_file") }
    }

    /// Writes bytes to the end of a file, creating it if it is missing.
    ///
    /// * `path` - the path to the file
    /// * `data` - the data to append
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn append_file_text(
        &self,
        ctx: Ctx,
        path: PathBuf,
        data: String,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("append_file_text") }
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
    fn read_dir(
        &self,
        ctx: Ctx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> impl Future<Output = io::Result<(Vec<DirEntry>, Vec<io::Error>)>> + Send {
        async { unsupported("read_dir") }
    }

    /// Creates a directory.
    ///
    /// * `path` - the path to the directory
    /// * `all` - if true, will create all missing parent components
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn create_dir(
        &self,
        ctx: Ctx,
        path: PathBuf,
        all: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("create_dir") }
    }

    /// Copies some file or directory.
    ///
    /// * `src` - the path to the file or directory to copy
    /// * `dst` - the path where the copy will be placed
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn copy(
        &self,
        ctx: Ctx,
        src: PathBuf,
        dst: PathBuf,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("copy") }
    }

    /// Removes some file or directory.
    ///
    /// * `path` - the path to a file or directory
    /// * `force` - if true, will remove non-empty directories
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn remove(
        &self,
        ctx: Ctx,
        path: PathBuf,
        force: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("remove") }
    }

    /// Renames some file or directory.
    ///
    /// * `src` - the path to the file or directory to rename
    /// * `dst` - the new name for the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn rename(
        &self,
        ctx: Ctx,
        src: PathBuf,
        dst: PathBuf,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("rename") }
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
    fn watch(
        &self,
        ctx: Ctx,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("watch") }
    }

    /// Removes a file or directory from being watched.
    ///
    /// * `path` - the path to the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn unwatch(&self, ctx: Ctx, path: PathBuf) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("unwatch") }
    }

    /// Checks if the specified path exists.
    ///
    /// * `path` - the path to the file or directory
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn exists(&self, ctx: Ctx, path: PathBuf) -> impl Future<Output = io::Result<bool>> + Send {
        async { unsupported("exists") }
    }

    /// Reads metadata for a file or directory.
    ///
    /// * `path` - the path to the file or directory
    /// * `canonicalize` - if true, will include a canonicalized path in the metadata
    /// * `resolve_file_type` - if true, will resolve symlinks to underlying type (file or dir)
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn metadata(
        &self,
        ctx: Ctx,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> impl Future<Output = io::Result<Metadata>> + Send {
        async { unsupported("metadata") }
    }

    /// Sets permissions for a file, directory, or symlink.
    ///
    /// * `path` - the path to the file, directory, or symlink
    /// * `resolve_symlink` - if true, will resolve the path to the underlying file/directory
    /// * `permissions` - the new permissions to apply
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn set_permissions(
        &self,
        ctx: Ctx,
        path: PathBuf,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("set_permissions") }
    }

    /// Searches files for matches based on a query.
    ///
    /// * `query` - the specific query to perform
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn search(
        &self,
        ctx: Ctx,
        query: SearchQuery,
    ) -> impl Future<Output = io::Result<SearchId>> + Send {
        async { unsupported("search") }
    }

    /// Cancels an actively-ongoing search.
    ///
    /// * `id` - the id of the search to cancel
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn cancel_search(&self, ctx: Ctx, id: SearchId) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("cancel_search") }
    }

    /// Spawns a new process, returning its id.
    ///
    /// * `cmd` - the full command to run as a new process (including arguments)
    /// * `environment` - the environment variables to associate with the process
    /// * `current_dir` - the alternative current directory to use with the process
    /// * `pty` - if provided, will run the process within a PTY of the given size
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn proc_spawn(
        &self,
        ctx: Ctx,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> impl Future<Output = io::Result<ProcessId>> + Send {
        async { unsupported("proc_spawn") }
    }

    /// Kills a running process by its id.
    ///
    /// * `id` - the unique id of the process
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn proc_kill(&self, ctx: Ctx, id: ProcessId) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("proc_kill") }
    }

    /// Sends data to the stdin of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `data` - the bytes to send to stdin
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn proc_stdin(
        &self,
        ctx: Ctx,
        id: ProcessId,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("proc_stdin") }
    }

    /// Resizes the PTY of the process with the specified id.
    ///
    /// * `id` - the unique id of the process
    /// * `size` - the new size of the pty
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn proc_resize_pty(
        &self,
        ctx: Ctx,
        id: ProcessId,
        size: PtySize,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async { unsupported("proc_resize_pty") }
    }

    /// Retrieves information about the system.
    ///
    /// *Override this, otherwise it will return "unsupported" as an error.*
    #[allow(unused_variables)]
    fn system_info(&self, ctx: Ctx) -> impl Future<Output = io::Result<SystemInfo>> + Send {
        async { unsupported("system_info") }
    }
}

impl<T> ServerHandler for ApiServerHandler<T>
where
    T: Api + Send + Sync + 'static,
{
    type Request = protocol::Msg<protocol::Request>;
    type Response = protocol::Msg<protocol::Response>;

    /// Overridden to leverage [`Api`] implementation of `on_connect`.
    async fn on_connect(&self, id: ConnectionId) -> io::Result<()> {
        T::on_connect(&self.api, id).await
    }

    /// Overridden to leverage [`Api`] implementation of `on_disconnect`.
    async fn on_disconnect(&self, id: ConnectionId) -> io::Result<()> {
        T::on_disconnect(&self.api, id).await
    }

    async fn on_request(&self, ctx: RequestCtx<Self::Request, Self::Response>) {
        let RequestCtx {
            connection_id,
            request,
            reply,
        } = ctx;

        // Convert our reply to a queued reply so we can ensure that the result
        // of an API function is sent back before anything else
        let reply = reply.queue();

        // Process single vs batch requests
        let response = match request.payload {
            protocol::Msg::Single(data) => {
                let ctx = Ctx {
                    connection_id,
                    reply: Box::new(SingleReply::from(reply.clone_reply())),
                };

                let data = handle_request(Arc::clone(&self.api), ctx, data).await;

                // Report outgoing errors in our debug logs
                if let protocol::Response::Error(x) = &data {
                    debug!("[Conn {}] {}", connection_id, x);
                }

                protocol::Msg::Single(data)
            }
            protocol::Msg::Batch(list)
                if matches!(request.header.get_as("sequence"), Some(Ok(true))) =>
            {
                let mut out = Vec::new();
                let mut has_failed = false;

                for data in list {
                    // Once we hit a failure, all remaining requests return interrupted
                    if has_failed {
                        out.push(protocol::Response::Error(protocol::Error {
                            kind: protocol::ErrorKind::Interrupted,
                            description: String::from("Canceled due to earlier error"),
                        }));
                        continue;
                    }

                    let ctx = Ctx {
                        connection_id,
                        reply: Box::new(SingleReply::from(reply.clone_reply())),
                    };

                    let data = handle_request(Arc::clone(&self.api), ctx, data).await;

                    // Report outgoing errors in our debug logs and mark as failed
                    // to cancel any future tasks being run
                    if let protocol::Response::Error(x) = &data {
                        debug!("[Conn {}] {}", connection_id, x);
                        has_failed = true;
                    }

                    out.push(data);
                }

                protocol::Msg::Batch(out)
            }
            protocol::Msg::Batch(list) => {
                let mut tasks = Vec::new();

                // If sequence specified as true, we want to process in order, otherwise we can
                // process in any order

                for data in list {
                    let api = Arc::clone(&self.api);
                    let ctx = Ctx {
                        connection_id,
                        reply: Box::new(SingleReply::from(reply.clone_reply())),
                    };

                    let task = tokio::spawn(async move {
                        let data = handle_request(api, ctx, data).await;

                        // Report outgoing errors in our debug logs
                        if let protocol::Response::Error(x) = &data {
                            debug!("[Conn {}] {}", connection_id, x);
                        }

                        data
                    });

                    tasks.push(task);
                }

                let out = futures::future::join_all(tasks)
                    .await
                    .into_iter()
                    .map(|x| match x {
                        Ok(x) => x,
                        Err(x) => protocol::Response::Error(x.to_string().into()),
                    })
                    .collect();
                protocol::Msg::Batch(out)
            }
        };

        // Queue up our result to go before ANY of the other messages that might be sent.
        // This is important to avoid situations such as when a process is started, but before
        // the confirmation can be sent some stdout or stderr is captured and sent first.
        if let Err(x) = reply.send_before(response) {
            error!("[Conn {}] Failed to send response: {}", connection_id, x);
        }

        // Flush out all of our replies thus far and toggle to no longer hold submissions
        if let Err(x) = reply.flush(false) {
            error!(
                "[Conn {}] Failed to flush response queue: {}",
                connection_id, x
            );
        }
    }
}

/// Processes an incoming request
async fn handle_request<T>(api: Arc<T>, ctx: Ctx, request: protocol::Request) -> protocol::Response
where
    T: Api + Send + Sync,
{
    match request {
        protocol::Request::Version {} => api
            .version(ctx)
            .await
            .map(protocol::Response::Version)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileRead { path } => api
            .read_file(ctx, path)
            .await
            .map(|data| protocol::Response::Blob { data })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileReadText { path } => api
            .read_file_text(ctx, path)
            .await
            .map(|data| protocol::Response::Text { data })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileWrite { path, data } => api
            .write_file(ctx, path, data)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileWriteText { path, text } => api
            .write_file_text(ctx, path, text)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileAppend { path, data } => api
            .append_file(ctx, path, data)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::FileAppendText { path, text } => api
            .append_file_text(ctx, path, text)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::DirRead {
            path,
            depth,
            absolute,
            canonicalize,
            include_root,
        } => api
            .read_dir(ctx, path, depth, absolute, canonicalize, include_root)
            .await
            .map(|(entries, errors)| protocol::Response::DirEntries {
                entries,
                errors: errors.into_iter().map(Error::from).collect(),
            })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::DirCreate { path, all } => api
            .create_dir(ctx, path, all)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Remove { path, force } => api
            .remove(ctx, path, force)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Copy { src, dst } => api
            .copy(ctx, src, dst)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Rename { src, dst } => api
            .rename(ctx, src, dst)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Watch {
            path,
            recursive,
            only,
            except,
        } => api
            .watch(ctx, path, recursive, only, except)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Unwatch { path } => api
            .unwatch(ctx, path)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Exists { path } => api
            .exists(ctx, path)
            .await
            .map(|value| protocol::Response::Exists { value })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Metadata {
            path,
            canonicalize,
            resolve_file_type,
        } => api
            .metadata(ctx, path, canonicalize, resolve_file_type)
            .await
            .map(protocol::Response::Metadata)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::SetPermissions {
            path,
            permissions,
            options,
        } => api
            .set_permissions(ctx, path, permissions, options)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::Search { query } => api
            .search(ctx, query)
            .await
            .map(|id| protocol::Response::SearchStarted { id })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::CancelSearch { id } => api
            .cancel_search(ctx, id)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::ProcSpawn {
            cmd,
            environment,
            current_dir,
            pty,
        } => api
            .proc_spawn(ctx, cmd.into(), environment, current_dir, pty)
            .await
            .map(|id| protocol::Response::ProcSpawned { id })
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::ProcKill { id } => api
            .proc_kill(ctx, id)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::ProcStdin { id, data } => api
            .proc_stdin(ctx, id, data)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::ProcResizePty { id, size } => api
            .proc_resize_pty(ctx, id, size)
            .await
            .map(|_| protocol::Response::Ok)
            .unwrap_or_else(protocol::Response::from),
        protocol::Request::SystemInfo {} => api
            .system_info(ctx)
            .await
            .map(protocol::Response::SystemInfo)
            .unwrap_or_else(protocol::Response::from),
    }
}
