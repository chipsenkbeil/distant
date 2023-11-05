use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use distant_core_protocol::{Error, Request, Response};
use tokio::sync::mpsc;

use crate::api::{
    Api, Ctx, FileSystemApi, ProcessApi, SearchApi, SystemInfoApi, VersionApi, WatchApi,
};

/// Full API for a distant-compatible client.
#[async_trait]
pub trait Client {
    /// Sends a request and returns a stream of responses, failing if unable to send a request or
    /// if the session's receiving line to the remote server has already been severed.
    async fn mail(&mut self, request: Request) -> io::Result<mpsc::UnboundedReceiver<Response>>;

    /// Sends a request and waits for a response, failing if unable to send a request or if
    /// the session's receiving line to the remote server has already been severed.
    async fn send(&mut self, request: Request) -> io::Result<Response> {
        let mut rx = self.mail(request).await?;
        rx.recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Channel has closed"))
    }

    /// Sends a request without waiting for a response; this method is able to be used even
    /// if the session's receiving line to the remote server has been severed.
    async fn fire(&mut self, request: Request) -> io::Result<()> {
        let _ = self.send(request).await?;
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
    async fn mail(&mut self, request: Request) -> io::Result<mpsc::UnboundedReceiver<Response>> {
        #[derive(Clone, Debug)]
        struct __Ctx(u32, mpsc::UnboundedSender<Response>);

        impl Ctx for __Ctx {
            fn connection(&self) -> u32 {
                self.0
            }

            fn clone_ctx(&self) -> Box<dyn Ctx> {
                Box::new(__Ctx(self.0, self.1.clone()))
            }

            fn send(&self, response: Response) -> io::Result<()> {
                self.1
                    .send(response)
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "Bridge has closed"))
            }
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let ctx = Box::new(__Ctx(rand::random(), tx));

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

        Ok(rx)
    }
}

/// Processes an incoming request.
async fn handle_request<T>(api: Arc<T>, ctx: Box<dyn Ctx>, request: Request) -> Response
where
    T: Api,
{
    match request {
        Request::Version {} => {
            let api = api.version();
            api.version(ctx)
                .await
                .map(Response::Version)
                .unwrap_or_else(Response::from)
        }
        Request::FileRead { path } => {
            let api = api.file_system();
            api.read_file(ctx, path)
                .await
                .map(|data| Response::Blob { data })
                .unwrap_or_else(Response::from)
        }
        Request::FileReadText { path } => {
            let api = api.file_system();
            api.read_file_text(ctx, path)
                .await
                .map(|data| Response::Text { data })
                .unwrap_or_else(Response::from)
        }
        Request::FileWrite { path, data } => {
            let api = api.file_system();
            api.write_file(ctx, path, data)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::FileWriteText { path, text } => {
            let api = api.file_system();
            api.write_file_text(ctx, path, text)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::FileAppend { path, data } => {
            let api = api.file_system();
            api.append_file(ctx, path, data)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::FileAppendText { path, text } => {
            let api = api.file_system();
            api.append_file_text(ctx, path, text)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::DirRead {
            path,
            depth,
            absolute,
            canonicalize,
            include_root,
        } => {
            let api = api.file_system();
            api.read_dir(ctx, path, depth, absolute, canonicalize, include_root)
                .await
                .map(|(entries, errors)| Response::DirEntries {
                    entries,
                    errors: errors.into_iter().map(Error::from).collect(),
                })
                .unwrap_or_else(Response::from)
        }
        Request::DirCreate { path, all } => {
            let api = api.file_system();
            api.create_dir(ctx, path, all)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Remove { path, force } => {
            let api = api.file_system();
            api.remove(ctx, path, force)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Copy { src, dst } => {
            let api = api.file_system();
            api.copy(ctx, src, dst)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Rename { src, dst } => {
            let api = api.file_system();
            api.rename(ctx, src, dst)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Watch {
            path,
            recursive,
            only,
            except,
        } => {
            let api = api.watch();
            api.watch(ctx, path, recursive, only, except)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Unwatch { path } => {
            let api = api.watch();
            api.unwatch(ctx, path)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Exists { path } => {
            let api = api.file_system();
            api.exists(ctx, path)
                .await
                .map(|value| Response::Exists { value })
                .unwrap_or_else(Response::from)
        }
        Request::Metadata {
            path,
            canonicalize,
            resolve_file_type,
        } => {
            let api = api.file_system();
            api.metadata(ctx, path, canonicalize, resolve_file_type)
                .await
                .map(Response::Metadata)
                .unwrap_or_else(Response::from)
        }
        Request::SetPermissions {
            path,
            permissions,
            options,
        } => {
            let api = api.file_system();
            api.set_permissions(ctx, path, permissions, options)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::Search { query } => {
            let api = api.search();
            api.search(ctx, query)
                .await
                .map(|id| Response::SearchStarted { id })
                .unwrap_or_else(Response::from)
        }
        Request::CancelSearch { id } => {
            let api = api.search();
            api.cancel_search(ctx, id)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::ProcSpawn {
            cmd,
            environment,
            current_dir,
            pty,
        } => {
            let api = api.process();
            api.proc_spawn(ctx, cmd.into(), environment, current_dir, pty)
                .await
                .map(|id| Response::ProcSpawned { id })
                .unwrap_or_else(Response::from)
        }
        Request::ProcKill { id } => {
            let api = api.process();
            api.proc_kill(ctx, id)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::ProcStdin { id, data } => {
            let api = api.process();
            api.proc_stdin(ctx, id, data)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::ProcResizePty { id, size } => {
            let api = api.process();
            api.proc_resize_pty(ctx, id, size)
                .await
                .map(|_| Response::Ok)
                .unwrap_or_else(Response::from)
        }
        Request::SystemInfo {} => {
            let api = api.system_info();
            api.system_info(ctx)
                .await
                .map(Response::SystemInfo)
                .unwrap_or_else(Response::from)
        }
    }
}
