use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::api::{
    Api, Ctx, FileSystemApi, ProcessApi, SearchApi, SystemInfoApi, VersionApi, WatchApi,
};
use crate::common::{Request, Response, Stream};
use crate::protocol;

mod err;
mod ext;

pub use err::{ClientError, ClientResult};
pub use ext::ClientExt;

/// Full API for a distant-compatible client.
#[async_trait]
pub trait Client {
    /// Sends a request and returns a stream of responses, failing if unable to send a request or
    /// if the session's receiving line to the remote server has already been severed.
    async fn send(&mut self, request: Request) -> io::Result<Box<dyn Stream<Item = Response>>>;

    /// Sends a request and waits for a single response, failing if unable to send a request or if
    /// the session's receiving line to the remote server has already been severed.
    async fn ask(&mut self, request: Request) -> io::Result<Response> {
        self.send(request)
            .await?
            .next()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Stream has closed"))
    }

    /// Sends a request without waiting for any response; this method is able to be used even
    /// if the session's receiving line to the remote server has been severed.
    async fn fire(&mut self, request: Request) -> io::Result<()> {
        let _ = self.ask(request).await?;
        Ok(())
    }
}

/// Represents a bridge between a [`Client`] and an [`Api`] implementation that maps requests to
/// the API and forwards responses back.
///
/// This can be used to run an Api implementation locally, such as when you want to translate some
/// other platform (e.g. ssh, docker) into a distant-compatible form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientBridge<T: Api> {
    api: Arc<T>,
}

impl<T: Api> ClientBridge<T> {
    /// Creates a new bridge wrapping around the provided api.
    pub fn new(api: T) -> Self {
        Self { api: Arc::new(api) }
    }
}

#[async_trait]
impl<T: Api + 'static> Client for ClientBridge<T> {
    async fn send(&mut self, request: Request) -> io::Result<Box<dyn Stream<Item = Response>>> {
        struct __Ctx(u32, mpsc::UnboundedSender<Response>);

        impl Ctx for __Ctx {
            fn connection(&self) -> u32 {
                self.0
            }

            fn clone_ctx(&self) -> Box<dyn Ctx> {
                Box::new(__Ctx(self.0, self.1.clone()))
            }

            fn send(&self, msg: protocol::Msg<protocol::Response>) -> io::Result<()> {
                self.1
                    .send(msg)
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "Bridge has closed"))
            }
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let ctx = Box::new(__Ctx(rand::random(), tx));

        // Spawn a task that will perform the request using the api. We use a task to allow the
        // async engine to not get blocked awaiting immediately for the response to arrive before
        // returning the stream of responses.
        tokio::task::spawn({
            let api = Arc::clone(&self.api);
            async move {
                let response = {
                    let ctx = ctx.clone_ctx();
                    handle_request(api, ctx, request).await
                };

                let _ = ctx.send(response);
            }
        });

        Ok(Box::new(rx))
    }
}

async fn handle_request<T>(api: Arc<T>, ctx: Box<dyn Ctx>, request: Request) -> Response
where
    T: Api,
{
    let origin = request.id;
    let sequence = request.flags.sequence;

    Response {
        id: rand::random(),
        origin,
        payload: match request.payload {
            protocol::Msg::Single(request) => {
                protocol::Msg::Single(handle_protocol_request(api, ctx, request).await)
            }
            protocol::Msg::Batch(requests) if sequence => {
                let mut responses = Vec::new();
                for request in requests {
                    responses.push(
                        handle_protocol_request(Arc::clone(&api), ctx.clone_ctx(), request).await,
                    );
                }
                protocol::Msg::Batch(responses)
            }
            protocol::Msg::Batch(requests) => {
                let mut responses = Vec::new();
                for request in requests {
                    responses.push(
                        handle_protocol_request(Arc::clone(&api), ctx.clone_ctx(), request).await,
                    );
                }
                protocol::Msg::Batch(responses)
            }
        },
    }
}

/// Processes a singular protocol request using the provided api and ctx.
async fn handle_protocol_request<T>(
    api: Arc<T>,
    ctx: Box<dyn Ctx>,
    request: protocol::Request,
) -> protocol::Response
where
    T: Api,
{
    match request {
        protocol::Request::Version {} => {
            let api = api.version();
            api.version(ctx)
                .await
                .map(protocol::Response::Version)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileRead { path } => {
            let api = api.file_system();
            api.read_file(ctx, path)
                .await
                .map(|data| protocol::Response::Blob { data })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileReadText { path } => {
            let api = api.file_system();
            api.read_file_text(ctx, path)
                .await
                .map(|data| protocol::Response::Text { data })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileWrite { path, data } => {
            let api = api.file_system();
            api.write_file(ctx, path, data)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileWriteText { path, text } => {
            let api = api.file_system();
            api.write_file_text(ctx, path, text)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileAppend { path, data } => {
            let api = api.file_system();
            api.append_file(ctx, path, data)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::FileAppendText { path, text } => {
            let api = api.file_system();
            api.append_file_text(ctx, path, text)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::DirRead {
            path,
            depth,
            absolute,
            canonicalize,
            include_root,
        } => {
            let api = api.file_system();
            api.read_dir(ctx, path, depth, absolute, canonicalize, include_root)
                .await
                .map(|(entries, errors)| protocol::Response::DirEntries {
                    entries,
                    errors: errors.into_iter().map(protocol::Error::from).collect(),
                })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::DirCreate { path, all } => {
            let api = api.file_system();
            api.create_dir(ctx, path, all)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Remove { path, force } => {
            let api = api.file_system();
            api.remove(ctx, path, force)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Copy { src, dst } => {
            let api = api.file_system();
            api.copy(ctx, src, dst)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Rename { src, dst } => {
            let api = api.file_system();
            api.rename(ctx, src, dst)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Watch {
            path,
            recursive,
            only,
            except,
        } => {
            let api = api.watch();
            api.watch(ctx, path, recursive, only, except)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Unwatch { path } => {
            let api = api.watch();
            api.unwatch(ctx, path)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Exists { path } => {
            let api = api.file_system();
            api.exists(ctx, path)
                .await
                .map(|value| protocol::Response::Exists { value })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Metadata {
            path,
            canonicalize,
            resolve_file_type,
        } => {
            let api = api.file_system();
            api.metadata(ctx, path, canonicalize, resolve_file_type)
                .await
                .map(protocol::Response::Metadata)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::SetPermissions {
            path,
            permissions,
            options,
        } => {
            let api = api.file_system();
            api.set_permissions(ctx, path, permissions, options)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::Search { query } => {
            let api = api.search();
            api.search(ctx, query)
                .await
                .map(|id| protocol::Response::SearchStarted { id })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::CancelSearch { id } => {
            let api = api.search();
            api.cancel_search(ctx, id)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::ProcSpawn {
            cmd,
            environment,
            current_dir,
            pty,
        } => {
            let api = api.process();
            api.proc_spawn(ctx, cmd.into(), environment, current_dir, pty)
                .await
                .map(|id| protocol::Response::ProcSpawned { id })
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::ProcKill { id } => {
            let api = api.process();
            api.proc_kill(ctx, id)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::ProcStdin { id, data } => {
            let api = api.process();
            api.proc_stdin(ctx, id, data)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::ProcResizePty { id, size } => {
            let api = api.process();
            api.proc_resize_pty(ctx, id, size)
                .await
                .map(|_| protocol::Response::Ok)
                .unwrap_or_else(protocol::Response::from)
        }
        protocol::Request::SystemInfo {} => {
            let api = api.system_info();
            api.system_info(ctx)
                .await
                .map(protocol::Response::SystemInfo)
                .unwrap_or_else(protocol::Response::from)
        }
    }
}
