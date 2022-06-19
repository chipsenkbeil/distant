use crate::{
    data::{ChangeKind, DirEntry, Error, Metadata, PtySize, SystemInfo},
    DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use derive_more::From;
use distant_net::{Reply, Server, ServerCtx};
use log::*;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, sync::Arc};

mod local;
pub use local::LocalDistantApi;

mod reply;
use reply::DistantSingleReply;

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub struct DistantCtx<T> {
    pub connection_id: usize,
    pub reply: Box<dyn Reply<Data = DistantResponseData>>,
    pub local_data: Arc<T>,
}

/// Represents a wrapper around a distant message, supporting single and batch requests
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DistantMsg<T> {
    Single(T),
    Batch(Vec<T>),
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

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
#[async_trait]
pub trait DistantApi {
    type LocalData: Send + Sync;

    async fn on_connection(&self, local_data: Self::LocalData) -> Self::LocalData {
        local_data
    }

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
    D: Send + Sync,
{
    type Request = DistantMsg<DistantRequestData>;
    type Response = DistantMsg<DistantResponseData>;
    type LocalData = D;

    /// Overridden to leverage [`DistantApi`] implementation of `on_connection`
    async fn on_connection(&self, local_data: Self::LocalData) -> Self::LocalData {
        T::on_connection(&self.api, local_data).await
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
        let response = match ctx.request.payload {
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
                        local_data,
                    };

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
        DistantRequestData::ProcSpawn { cmd, persist, pty } => server
            .api
            .proc_spawn(ctx, cmd, persist, pty)
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
