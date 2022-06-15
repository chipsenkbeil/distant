use crate::{DistantRequestData, DistantResponseData};
use async_trait::async_trait;
use distant_net::{Server, ServerCtx};
use std::io;

/// Represents the context provided to the [`DistantApi`] for incoming requests
pub type DistantServerCtx = ServerCtx<DistantRequestData, DistantResponseData, (), ()>;

fn unsupported() -> io::Error {
    io::Error::from(io::ErrorKind::Unsupported)
}

/// Interface to support the suite of functionality available with distant,
/// which can be used to build other servers that are compatible with distant
#[async_trait]
pub trait DistantApi {
    /// Request to read a binary file
    async fn read_file(&self, ctx: DistantServerCtx) -> io::Result<()> {
        Err(unsupported())
    }
}

#[async_trait]
impl<T: DistantApi> Server for T {
    type Request = DistantRequestData;
    type Response = DistantResponseData;
    type GlobalData = ();
    type LocalData = ();

    async fn on_request(
        &self,
        ctx: ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
    ) {
        match ctx.request.payload {
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
}
