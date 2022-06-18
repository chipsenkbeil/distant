use crate::{
    api::local::ConnectionState,
    data::{ChangeKind, DirEntry, Error, Metadata, PtySize, SystemInfo},
    DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use distant_net::{QueuedServerReply, Server, ServerCtx};
use log::*;
use std::{io, path::PathBuf, sync::Arc};

mod local;
pub use local::LocalDistantApi;

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub struct DistantCtx<T> {
    pub connection_id: usize,
    pub reply: QueuedServerReply<DistantResponseData>,
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

impl Default for DistantApiServer<LocalDistantApi, ConnectionState> {
    fn default() -> Self {
        Self::new(LocalDistantApi::new())
    }
}

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
#[async_trait]
pub trait DistantApi {
    type LocalData;

    async fn read_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<Vec<u8>>;
    async fn read_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<String>;
    async fn write_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()>;
    async fn write_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()>;
    async fn append_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()>;
    async fn append_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()>;
    async fn read_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)>;
    async fn create_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        all: bool,
    ) -> io::Result<()>;
    async fn copy(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()>;
    async fn remove(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        force: bool,
    ) -> io::Result<()>;
    async fn rename(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()>;
    async fn watch(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> io::Result<()>;
    async fn unwatch(&self, ctx: DistantCtx<Self::LocalData>, path: PathBuf) -> io::Result<()>;
    async fn exists(&self, ctx: DistantCtx<Self::LocalData>, path: PathBuf) -> io::Result<bool>;
    async fn metadata(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata>;
    async fn proc_spawn(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        cmd: String,
        persist: bool,
        pty: Option<PtySize>,
    ) -> io::Result<usize>;
    async fn proc_kill(&self, ctx: DistantCtx<Self::LocalData>, id: usize) -> io::Result<()>;
    async fn proc_stdin(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: usize,
        data: Vec<u8>,
    ) -> io::Result<()>;
    async fn proc_resize_pty(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: usize,
        size: PtySize,
    ) -> io::Result<()>;
    async fn system_info(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<SystemInfo>;
}

#[async_trait]
impl<T, D> Server for DistantApiServer<T, D>
where
    T: DistantApi<LocalData = D> + Send + Sync,
    D: Default + Send + Sync,
{
    type Request = DistantRequestData;
    type Response = DistantResponseData;
    type LocalData = D;

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            local_data,
        } = ctx;

        let reply = reply.queue();
        let ctx = DistantCtx {
            connection_id,
            reply: reply.clone(),
            local_data,
        };

        let response = match request.payload {
            DistantRequestData::FileRead { path } => self
                .api
                .read_file(ctx, path)
                .await
                .map(|data| DistantResponseData::Blob { data })
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::FileReadText { path } => self
                .api
                .read_file_text(ctx, path)
                .await
                .map(|data| DistantResponseData::Text { data })
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::FileWrite { path, data } => self
                .api
                .write_file(ctx, path, data)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::FileWriteText { path, text } => self
                .api
                .write_file_text(ctx, path, text)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::FileAppend { path, data } => self
                .api
                .append_file(ctx, path, data)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::FileAppendText { path, text } => self
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
            } => self
                .api
                .read_dir(ctx, path, depth, absolute, canonicalize, include_root)
                .await
                .map(|(entries, errors)| DistantResponseData::DirEntries {
                    entries,
                    errors: errors.into_iter().map(Error::from).collect(),
                })
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::DirCreate { path, all } => self
                .api
                .create_dir(ctx, path, all)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Remove { path, force } => self
                .api
                .remove(ctx, path, force)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Copy { src, dst } => self
                .api
                .copy(ctx, src, dst)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Rename { src, dst } => self
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
            } => self
                .api
                .watch(ctx, path, recursive, only, except)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Unwatch { path } => self
                .api
                .unwatch(ctx, path)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Exists { path } => self
                .api
                .exists(ctx, path)
                .await
                .map(|value| DistantResponseData::Exists { value })
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => self
                .api
                .metadata(ctx, path, canonicalize, resolve_file_type)
                .await
                .map(DistantResponseData::Metadata)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::ProcSpawn { cmd, persist, pty } => self
                .api
                .proc_spawn(ctx, cmd, persist, pty)
                .await
                .map(|id| DistantResponseData::ProcSpawned { id })
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::ProcKill { id } => self
                .api
                .proc_kill(ctx, id)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::ProcStdin { id, data } => self
                .api
                .proc_stdin(ctx, id, data)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),

            DistantRequestData::ProcResizePty { id, size } => self
                .api
                .proc_resize_pty(ctx, id, size)
                .await
                .map(|_| DistantResponseData::Ok)
                .unwrap_or_else(DistantResponseData::from),
            DistantRequestData::SystemInfo {} => self
                .api
                .system_info(ctx)
                .await
                .map(DistantResponseData::SystemInfo)
                .unwrap_or_else(DistantResponseData::from),
        };

        // Queue up our result to go before ANY of the other messages that might be sent.
        // This is important to avoid situations where a process is started, but before
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
