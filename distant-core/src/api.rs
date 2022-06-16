use crate::{
    data::{ChangeKind, DirEntry, Metadata, PtySize},
    DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use distant_net::{QueuedServerReply, Server, ServerCtx};
use std::{io, path::PathBuf};

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub struct DistantCtx {
    pub reply: QueuedServerReply<DistantResponseData>,
}

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
#[async_trait]
pub trait DistantApi {
    async fn read_file(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<Vec<u8>>;
    async fn read_file_text(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<String>;
    async fn write_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()>;
    async fn write_file_text(&self, ctx: DistantCtx, path: PathBuf, data: String)
        -> io::Result<()>;
    async fn append_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()>;
    async fn append_file_text(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        data: String,
    ) -> io::Result<()>;
    async fn read_dir(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<Vec<DirEntry>>;
    async fn create_dir(&self, ctx: DistantCtx, path: PathBuf, all: bool) -> io::Result<()>;
    async fn remove(&self, ctx: DistantCtx, path: PathBuf, force: bool) -> io::Result<()>;
    async fn rename(&self, ctx: DistantCtx, src: PathBuf, dst: PathBuf) -> io::Result<()>;
    async fn watch(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> io::Result<()>;
    async fn unwatch(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<()>;
    async fn exists(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<bool>;
    async fn metadata(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata>;
    async fn proc_spawn(
        &self,
        ctx: DistantCtx,
        cmd: String,
        persist: bool,
        pty: Option<PtySize>,
    ) -> io::Result<usize>;
    async fn proc_kill(&self, ctx: DistantCtx, id: usize) -> io::Result<()>;
    async fn proc_stdin(&self, ctx: DistantCtx, id: usize, data: Vec<u8>) -> io::Result<()>;
    async fn proc_resize_pty(&self, ctx: DistantCtx, id: usize, size: PtySize) -> io::Result<()>;
    async fn proc_list(&self, ctx: DistantCtx) -> io::Result<()>;
    async fn system_info(&self, ctx: DistantCtx) -> io::Result<()>;
}

#[async_trait]
impl<T: DistantApi> Server for T {
    type Request = DistantRequestData;
    type Response = DistantResponseData;
    type LocalData = ();

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            state,
        } = ctx;

        let ctx = DistantCtx {
            reply: reply.queue(),
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

/* /// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    conn_id: usize,
    state: HState,
    req: Request,
    tx: mpsc::Sender<Response>,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner<F>(
        conn_id: usize,
        state: HState,
        data: DistantRequestData,
        reply: F,
    ) -> Result<Outgoing, ServerError>
    where
        F: FnMut(Vec<DistantResponseData>) -> ReplyRet + Clone + Send + 'static,
    {
        match data {
            DistantRequestData::FileRead { path } => file_read(path).await,
            DistantRequestData::FileReadText { path } => file_read_text(path).await,
            DistantRequestData::FileWrite { path, data } => file_write(path, data).await,
            DistantRequestData::FileWriteText { path, text } => file_write(path, text).await,
            DistantRequestData::FileAppend { path, data } => file_append(path, data).await,
            DistantRequestData::FileAppendText { path, text } => file_append(path, text).await,
            DistantRequestData::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => dir_read(path, depth, absolute, canonicalize, include_root).await,
            DistantRequestData::DirCreate { path, all } => dir_create(path, all).await,
            DistantRequestData::Remove { path, force } => remove(path, force).await,
            DistantRequestData::Copy { src, dst } => copy(src, dst).await,
            DistantRequestData::Rename { src, dst } => rename(src, dst).await,
            DistantRequestData::Watch {
                path,
                recursive,
                only,
                except,
            } => watch(conn_id, state, reply, path, recursive, only, except).await,
            DistantRequestData::Unwatch { path } => unwatch(conn_id, state, path).await,
            DistantRequestData::Exists { path } => exists(path).await,
            DistantRequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => metadata(path, canonicalize, resolve_file_type).await,
            DistantRequestData::ProcSpawn { cmd, persist, pty } => {
                proc_spawn(conn_id, state, reply, cmd, persist, pty).await
            }
            DistantRequestData::ProcKill { id } => proc_kill(conn_id, state, id).await,
            DistantRequestData::ProcStdin { id, data } => {
                proc_stdin(conn_id, state, id, data).await
            }
            DistantRequestData::ProcResizePty { id, size } => {
                proc_resize_pty(conn_id, state, id, size).await
            }
            DistantRequestData::ProcList {} => proc_list(state).await,
            DistantRequestData::SystemInfo {} => system_info().await,
        }
    }

    let reply = {
        let origin_id = req.id;
        let tenant = req.tenant.clone();
        let tx_2 = tx.clone();
        move |payload: Vec<DistantResponseData>| -> ReplyRet {
            let tx = tx_2.clone();
            let res = Response::new(tenant.to_string(), origin_id, payload);
            Box::pin(async move { tx.send(res).await.is_ok() })
        }
    };

    // Build up a collection of tasks to run independently
    let mut payload_tasks = Vec::new();
    for data in req.payload {
        let state_2 = Arc::clone(&state);
        let reply_2 = reply.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(conn_id, state_2, data, reply_2).await {
                Ok(outgoing) => outgoing,
                Err(x) => Outgoing::from(DistantResponseData::from(x)),
            }
        }));
    }

    // Collect the results of our tasks into the payload entries
    let mut outgoing: Vec<Outgoing> = future::join_all(payload_tasks)
        .await
        .into_iter()
        .map(|x| match x {
            Ok(outgoing) => outgoing,
            Err(x) => Outgoing::from(DistantResponseData::from(x)),
        })
        .collect();

    let post_hooks: Vec<PostHook> = outgoing
        .iter_mut()
        .filter_map(|x| x.post_hook.take())
        .collect();

    let payload = outgoing.into_iter().map(|x| x.data).collect();
    let res = Response::new(req.tenant, req.id, payload);

    // Send out our primary response from processing the request
    let result = tx.send(res).await;

    // Invoke all post hooks
    for hook in post_hooks {
        hook();
    }

    result
} */
