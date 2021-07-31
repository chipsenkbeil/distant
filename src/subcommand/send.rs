use crate::{
    data::{Request, RequestPayload, Response, ResponsePayload},
    net::{Client, TransportError},
    opt::{CommonOpt, SendMode, SendSubcommand},
    utils::{Session, SessionError},
};
use derive_more::{Display, Error, From};
use log::*;
use structopt::StructOpt;
use tokio::{
    io,
    sync::{
        mpsc,
        oneshot::{self, error::TryRecvError},
    },
};
use tokio_stream::StreamExt;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    SessionError(SessionError),
    TransportError(TransportError),

    #[display(fmt = "Non-interactive but no operation supplied")]
    MissingOperation,
}

pub fn run(cmd: SendSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: SendSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let session = Session::load().await?;
    let mut client = Client::connect(session).await?;

    if !cmd.interactive && cmd.operation.is_none() {
        return Err(Error::MissingOperation);
    }

    // Special conditions for continuing to process responses
    let mut is_proc_req = false;
    let mut proc_id = 0;

    if let Some(req) = cmd.operation.map(Request::from) {
        is_proc_req = req.payload.is_proc_run();

        let res = client.send(req).await?;

        // Store the spawned process id for using in sending stdin (if we spawned a proc)
        proc_id = match &res.payload {
            ResponsePayload::ProcStart { id } => *id,
            _ => 0,
        };

        format_response(cmd.mode, res)?.print();
    }

    // If we are executing a process, we want to continue interacting via stdin and receiving
    // results via stdout/stderr
    //
    // If we are interactive, we want to continue looping regardless
    if is_proc_req || cmd.interactive {
        interactive_loop(client, proc_id, cmd.mode, cmd.interactive).await?;
    }

    Ok(())
}

fn spawn_stdin_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(1);

    // NOTE: Using blocking I/O per tokio's advice to read from stdin line-by-line and then
    //       pass the results to a separate async handler to forward to the remote process
    std::thread::spawn(move || {
        let stdin = std::io::stdin();

        loop {
            let mut line = String::new();
            if stdin.read_line(&mut line).is_ok() {
                if let Err(x) = tx.blocking_send(line) {
                    error!(
                        "Failed to pass along stdin to be sent to remote process: {}",
                        x
                    );
                }
            } else {
                break;
            }
        }
    });

    rx
}

async fn interactive_loop(
    mut client: Client,
    id: usize,
    mode: SendMode,
    interactive: bool,
) -> Result<(), Error> {
    let mut stream = client.to_response_stream();

    // Create a channel that can report when we should stop the loop based on a received request
    let (tx_stop, mut rx_stop) = oneshot::channel::<()>();

    // We also want to spawn a task to handle sending stdin to the remote process
    let mut rx = spawn_stdin_reader();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            match mode {
                // Special exit condition for interactive mode
                _ if line.trim() == "exit" => {
                    if let Err(_) = tx_stop.send(()) {
                        error!("Failed to close interactive loop!");
                    }
                    break;
                }

                // For json mode, all stdin is treated as individual requests
                SendMode::Json => {
                    trace!("Client sending request: {:?}", line);
                    let result = serde_json::from_str(&line)
                        .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x));
                    match result {
                        Ok(req) => match client.send(req).await {
                            Ok(res) => match format_response(mode, res) {
                                Ok(out) => out.print(),
                                Err(x) => error!("Failed to format response: {}", x),
                            },
                            Err(x) => {
                                error!("Failed to send request to remote process ({}): {}", id, x)
                            }
                        },
                        Err(x) => {
                            error!("Failed to serialize request: {}", x);
                        }
                    }
                }

                // For interactive shell mode, parse stdin as individual commands
                SendMode::Shell if interactive => {
                    if line.trim().is_empty() {
                        continue;
                    }

                    trace!("Client sending command: {:?}", line);

                    // NOTE: We have to stick something in as the first argument as clap/structopt
                    //       expect the binary name as the first item in the iterator
                    let payload_result = RequestPayload::from_iter_safe(
                        std::iter::once("distant")
                            .chain(line.trim().split(' ').filter(|s| !s.trim().is_empty())),
                    );
                    match payload_result {
                        Ok(payload) => match client.send(Request::from(payload)).await {
                            Ok(res) => match format_response(mode, res) {
                                Ok(out) => out.print(),
                                Err(x) => error!("Failed to format response: {}", x),
                            },
                            Err(x) => {
                                error!("Failed to send request to remote process ({}): {}", id, x)
                            }
                        },
                        Err(x) => {
                            error!("Failed to parse command: {}", x);
                        }
                    }
                }

                // For non-interactive shell mode, all stdin is treated as a proc's stdin
                SendMode::Shell => {
                    trace!("Client sending stdin: {:?}", line);
                    let req = Request::from(RequestPayload::ProcStdin {
                        id,
                        data: line.into_bytes(),
                    });
                    let result = client.send(req).await;

                    if let Err(x) = result {
                        error!("Failed to send stdin to remote process ({}): {}", id, x);
                    }
                }
            }
        }
    });

    while let Err(TryRecvError::Empty) = rx_stop.try_recv() {
        if let Some(res) = stream.next().await {
            let res = res.map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Response stream no longer available",
                )
            })?;
            let done = res.payload.is_proc_done() && !interactive;

            format_response(mode, res)?.print();

            // If we aren't interactive but are just running a proc and
            // we've received the end of the proc, we should exit
            if done {
                break;
            }

        // If we have nothing else in our stream, we should also exit
        } else {
            break;
        }
    }

    Ok(())
}

/// Represents the output content and destination
enum ResponseOut {
    Stdout(String),
    Stderr(String),
    None,
}

impl ResponseOut {
    pub fn print(self) {
        match self {
            Self::Stdout(x) => print!("{}", x),
            Self::Stderr(x) => eprint!("{}", x),
            Self::None => {}
        }
    }
}

fn format_response(mode: SendMode, res: Response) -> io::Result<ResponseOut> {
    Ok(match mode {
        SendMode::Json => ResponseOut::Stdout(format!(
            "{}\n",
            serde_json::to_string(&res)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
        )),
        SendMode::Shell => format_shell(res),
    })
}

fn format_shell(res: Response) -> ResponseOut {
    match res.payload {
        ResponsePayload::Ok => ResponseOut::None,
        ResponsePayload::Error { description } => {
            ResponseOut::Stderr(format!("Failed: '{}'.\n", description))
        }
        ResponsePayload::Blob { data } => {
            ResponseOut::Stdout(String::from_utf8_lossy(&data).to_string())
        }
        ResponsePayload::Text { data } => ResponseOut::Stdout(data),
        ResponsePayload::DirEntries { entries } => ResponseOut::Stdout(format!(
            "{}\n",
            entries
                .into_iter()
                .map(|entry| {
                    format!(
                        "{}{}",
                        entry.path.as_os_str().to_string_lossy(),
                        if entry.file_type.is_dir() {
                            std::path::MAIN_SEPARATOR.to_string()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect::<Vec<String>>()
                .join("\n"),
        )),
        ResponsePayload::ProcEntries { entries } => ResponseOut::Stdout(format!(
            "{}\n",
            entries
                .into_iter()
                .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
                .collect::<Vec<String>>()
                .join("\n"),
        )),
        ResponsePayload::ProcStart { .. } => ResponseOut::None,
        ResponsePayload::ProcStdout { data, .. } => {
            ResponseOut::Stdout(String::from_utf8_lossy(&data).to_string())
        }
        ResponsePayload::ProcStderr { data, .. } => {
            ResponseOut::Stderr(String::from_utf8_lossy(&data).to_string())
        }
        ResponsePayload::ProcDone { id, success, code } => {
            if success {
                ResponseOut::None
            } else if let Some(code) = code {
                ResponseOut::Stderr(format!("Proc {} failed with code {}\n", id, code))
            } else {
                ResponseOut::Stderr(format!("Proc {} failed\n", id))
            }
        }
    }
}
