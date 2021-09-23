use async_compat::CompatExt;
use distant_core::{
    data::{DirEntry, FileType, RunningProcess},
    Request, RequestData, Response, ResponseData,
};
use futures::future;
use log::*;
use std::{
    collections::HashMap,
    future::Future,
    io::{self, Read, Write},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, Mutex, MutexGuard};
use wezterm_ssh::{Child, ExecResult, FileDescriptor, Session as WezSession, SshChildProcess};

const MAX_PIPE_CHUNK_SIZE: usize = 1024;
const READ_PAUSE_MILLIS: u64 = 50;
const WAIT_PAUSE_MILLIS: u64 = 50;

#[derive(Default)]
pub(crate) struct State {
    processes: HashMap<usize, Process>,
}

struct Process {
    id: usize,
    cmd: String,
    args: Vec<String>,
    stdin_tx: mpsc::Sender<String>,
    kill_tx: oneshot::Sender<()>,
}

type ReplyRet = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

type PostHook = Box<dyn FnOnce(MutexGuard<'_, State>) + Send>;
struct Outgoing {
    data: ResponseData,
    post_hook: Option<PostHook>,
}

impl From<ResponseData> for Outgoing {
    fn from(data: ResponseData) -> Self {
        Self {
            data,
            post_hook: None,
        }
    }
}

/// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    session: WezSession,
    state: Arc<Mutex<State>>,
    req: Request,
    tx: mpsc::Sender<Response>,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner<F>(
        session: WezSession,
        state: Arc<Mutex<State>>,
        data: RequestData,
        reply: F,
    ) -> io::Result<Outgoing>
    where
        F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
    {
        match data {
            RequestData::FileRead { path } => file_read(session, path).await,
            RequestData::FileReadText { path } => file_read_text(session, path).await,
            RequestData::FileWrite { path, data } => file_write(session, path, data).await,
            RequestData::FileWriteText { path, text } => file_write(session, path, text).await,
            RequestData::FileAppend { path, data } => file_append(session, path, data).await,
            RequestData::FileAppendText { path, text } => file_append(session, path, text).await,
            RequestData::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => dir_read(session, path, depth, absolute, canonicalize, include_root).await,
            RequestData::DirCreate { path, all } => dir_create(session, path, all).await,
            RequestData::Remove { path, force } => remove(session, path, force).await,
            RequestData::Copy { src, dst } => copy(session, src, dst).await,
            RequestData::Rename { src, dst } => rename(session, src, dst).await,
            RequestData::Exists { path } => exists(session, path).await,
            RequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => metadata(session, path, canonicalize, resolve_file_type).await,
            RequestData::ProcRun { cmd, args } => proc_run(session, state, reply, cmd, args).await,
            RequestData::ProcKill { id } => proc_kill(session, state, id).await,
            RequestData::ProcStdin { id, data } => proc_stdin(session, state, id, data).await,
            RequestData::ProcList {} => proc_list(session, state).await,
            RequestData::SystemInfo {} => system_info(session).await,
        }
    }

    let reply = {
        let origin_id = req.id;
        let tenant = req.tenant.clone();
        let tx_2 = tx.clone();
        move |payload: Vec<ResponseData>| -> ReplyRet {
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
        let session = session.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(session, state_2, data, reply_2).await {
                Ok(outgoing) => outgoing,
                Err(x) => Outgoing::from(ResponseData::from(x)),
            }
        }));
    }

    // Collect the results of our tasks into the payload entries
    let mut outgoing: Vec<Outgoing> = future::join_all(payload_tasks)
        .await
        .into_iter()
        .map(|x| match x {
            Ok(outgoing) => outgoing,
            Err(x) => Outgoing::from(ResponseData::from(x)),
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
        hook(state.lock().await);
    }

    result
}

async fn file_read(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    todo!();
}

async fn file_read_text(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    todo!();
}

async fn file_write(
    session: WezSession,
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> io::Result<Outgoing> {
    todo!();
}

async fn file_append(
    session: WezSession,
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> io::Result<Outgoing> {
    todo!();
}

async fn dir_read(
    session: WezSession,
    path: PathBuf,
    depth: usize,
    absolute: bool,
    canonicalize: bool,
    include_root: bool,
) -> io::Result<Outgoing> {
    todo!();
}

async fn dir_create(session: WezSession, path: PathBuf, all: bool) -> io::Result<Outgoing> {
    todo!();
}

async fn remove(session: WezSession, path: PathBuf, force: bool) -> io::Result<Outgoing> {
    todo!();
}

async fn copy(session: WezSession, src: PathBuf, dst: PathBuf) -> io::Result<Outgoing> {
    todo!();
}

async fn rename(session: WezSession, src: PathBuf, dst: PathBuf) -> io::Result<Outgoing> {
    todo!();
}

async fn exists(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    todo!();
}

async fn metadata(
    session: WezSession,
    path: PathBuf,
    canonicalize: bool,
    resolve_file_type: bool,
) -> io::Result<Outgoing> {
    todo!();
}

async fn proc_run<F>(
    session: WezSession,
    state: Arc<Mutex<State>>,
    reply: F,
    cmd: String,
    args: Vec<String>,
) -> io::Result<Outgoing>
where
    F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
{
    let id = rand::random();
    let cmd_string = format!("{} {}", cmd, args.join(" "));

    let ExecResult {
        mut stdin,
        mut stdout,
        mut stderr,
        mut child,
    } = session
        .exec(&cmd_string, None)
        .compat()
        .await
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

    let (stdin_tx, mut stdin_rx) = mpsc::channel(1);
    let (kill_tx, mut kill_rx) = oneshot::channel();
    state.lock().await.processes.insert(
        id,
        Process {
            id,
            cmd,
            args,
            stdin_tx,
            kill_tx,
        },
    );

    let post_hook = Box::new(move |state_lock: MutexGuard<'_, State>| {
        // Spawn a task that sends stdout as a response
        let mut reply_2 = reply.clone();
        let stdout_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stdout.read(&mut buf) {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStdout { id, data }];
                            if !reply_2(payload).await {
                                error!("<Ssh: Proc {}> Stdout channel closed", id);
                                break;
                            }

                            // Pause to allow buffer to fill up a little bit, avoiding
                            // spamming with a lot of smaller responses
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                READ_PAUSE_MILLIS,
                            ))
                            .await;
                        }
                        Err(x) => {
                            error!(
                                "<Ssh: Proc {}> Invalid data read from stdout pipe: {}",
                                id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn a task that sends stderr as a response
        let mut reply_2 = reply.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stderr.read(&mut buf) {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStderr { id, data }];
                            if !reply_2(payload).await {
                                error!("<Ssh: Proc {}> Stderr channel closed", id);
                                break;
                            }

                            // Pause to allow buffer to fill up a little bit, avoiding
                            // spamming with a lot of smaller responses
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                READ_PAUSE_MILLIS,
                            ))
                            .await;
                        }
                        Err(x) => {
                            error!(
                                "<Ssh: Proc {}> Invalid data read from stderr pipe: {}",
                                id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(_) => break,
                }
            }
        });

        let stdin_task = tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if let Err(x) = stdin.write_all(line.as_bytes()) {
                    error!("<Ssh: Proc {}> Failed to send stdin: {}", id, x);
                    break;
                }
            }
        });

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let state_2 = Arc::clone(&state);
        let mut reply_2 = reply.clone();
        let wait_task = tokio::spawn(async move {
            let (success, should_kill) = loop {
                match (child.try_wait(), kill_rx.try_recv()) {
                    (Ok(Some(status)), _) => break (status.success(), false),
                    (_, Ok(_) | Err(oneshot::error::TryRecvError::Closed)) => break (false, true),
                    _ => {}
                }

                tokio::time::sleep(Duration::from_millis(WAIT_PAUSE_MILLIS)).await;
            };

            // Force stdin task to abort if it hasn't exited as there is no
            // point to sending any more stdin
            stdin_task.abort();

            if should_kill {
                debug!("<Ssh: Proc {}> Process killed", id);
                if let Err(x) = child.kill() {
                    error!("<Ssh: Proc {}> Unable to kill process: {}", id, x);
                }
            } else {
                debug!("<Ssh: Proc {}> Process done", id);
            }

            if let Err(x) = stderr_task.await {
                error!("<Ssh: Proc {}> Join on stderr task failed: {}", id, x);
            }

            if let Err(x) = stdout_task.await {
                error!("<Ssh: Proc {}> Join on stdout task failed: {}", id, x);
            }

            state_2.lock().await.processes.remove(&id);

            let payload = vec![ResponseData::ProcDone {
                id,
                success: !should_kill && success,
                code: None,
            }];

            if !reply_2(payload).await {
                error!("<Ssh: Proc {}> Failed to send done!", id,);
            }
        });
    });

    Ok(Outgoing {
        data: ResponseData::ProcStart { id },
        post_hook: Some(post_hook),
    })
}

async fn proc_kill(
    session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.remove(&id) {
        if process.kill_tx.send(()).is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Unable to send kill signal to process",
    ))
}

async fn proc_stdin(
    session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
    data: String,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.get_mut(&id) {
        if process.stdin_tx.send(data).await.is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Unable to send stdin to process",
    ))
}

async fn proc_list(session: WezSession, state: Arc<Mutex<State>>) -> io::Result<Outgoing> {
    Ok(Outgoing::from(ResponseData::ProcEntries {
        entries: state
            .lock()
            .await
            .processes
            .values()
            .map(|p| RunningProcess {
                cmd: p.cmd.to_string(),
                args: p.args.clone(),
                id: p.id,
            })
            .collect(),
    }))
}

async fn system_info(session: WezSession) -> io::Result<Outgoing> {
    todo!();
}
