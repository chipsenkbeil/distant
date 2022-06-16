use crate::{
    data::{ChangeKind, DirEntry, Metadata, PtySize},
    DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use distant_net::{QueuedServerReply, Server, ServerCtx};
use std::{io, path::PathBuf, sync::Arc};

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub struct DistantCtx<T> {
    pub reply: QueuedServerReply<DistantResponseData>,
    pub local_data: Arc<T>,
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
    async fn proc_list(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<()>;
    async fn system_info(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<()>;
}

#[async_trait]
impl<T, D> Server for T
where
    T: DistantApi<LocalData = D>,
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

        let ctx = DistantCtx {
            reply: reply.queue(),
            local_data,
        };

        match request.payload {
            DistantRequestData::FileRead { path } => self.read_file(ctx, path).await,
            DistantRequestData::FileReadText { path } => self.read_file_text(ctx, path).await,
            DistantRequestData::FileWrite { path, data } => self.write_file(ctx, path, data).await,
            DistantRequestData::FileWriteText { path, text } => {
                self.write_file_text(ctx, path, text).await
            }
            DistantRequestData::FileAppend { path, data } => {
                self.append_file(ctx, path, data).await
            }
            DistantRequestData::FileAppendText { path, text } => {
                self.append_file_text(ctx, path, text).await
            }
            DistantRequestData::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => {
                self.read_dir(ctx, path, depth, absolute, canonicalize, include_root)
                    .await
            }
            DistantRequestData::DirCreate { path, all } => self.create_dir(ctx, path, all).await,
            DistantRequestData::Remove { path, force } => self.remove(ctx, path, force).await,
            DistantRequestData::Copy { src, dst } => self.copy(ctx, src, dst).await,
            DistantRequestData::Rename { src, dst } => self.rename(ctx, src, dst).await,
            DistantRequestData::Watch {
                path,
                recursive,
                only,
                except,
            } => self.watch(ctx, path, recursive, only, except).await,
            DistantRequestData::Unwatch { path } => self.unwatch(ctx, path).await,
            DistantRequestData::Exists { path } => self.exists(ctx, path).await,
            DistantRequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => {
                self.metadata(ctx, path, canonicalize, resolve_file_type)
                    .await
            }
            DistantRequestData::ProcSpawn { cmd, persist, pty } => {
                self.proc_spawn(ctx, cmd, persist, pty).await
            }
            DistantRequestData::ProcKill { id } => self.proc_kill(ctx, id).await,
            DistantRequestData::ProcStdin { id, data } => self.proc_stdin(ctx, id, data).await,
            DistantRequestData::ProcResizePty { id, size } => {
                self.proc_resize_pty(ctx, id, size).await
            }
            DistantRequestData::ProcList {} => self.proc_list(ctx).await,
            DistantRequestData::SystemInfo {} => self.system_info(ctx).await,
        }
    }
}
