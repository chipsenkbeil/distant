use crate::{
    cli::opt::Mode,
    core::{
        constants::MAX_PIPE_CHUNK_SIZE,
        data::{Request, RequestPayload, Response, ResponsePayload},
        net::{Client, DataStream},
        utils::StringBuf,
    },
};
use derive_more::IsVariant;
use log::*;
use std::marker::Unpin;
use structopt::StructOpt;
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::{
        mpsc,
        oneshot::{self, error::TryRecvError},
    },
};
use tokio_stream::StreamExt;

#[derive(Copy, Clone, PartialEq, Eq, IsVariant)]
pub enum LoopConfig {
    Json,
    Proc { id: usize },
    Shell,
}

impl From<LoopConfig> for Mode {
    fn from(config: LoopConfig) -> Self {
        match config {
            LoopConfig::Json => Self::Json,
            LoopConfig::Proc { .. } | LoopConfig::Shell => Self::Shell,
        }
    }
}

/// Starts a new action loop that processes requests and receives responses
///
/// id represents the id of a remote process
pub async fn interactive_loop<T>(
    mut client: Client<T>,
    tenant: String,
    config: LoopConfig,
) -> io::Result<()>
where
    T: AsyncRead + AsyncWrite + DataStream + Unpin + 'static,
{
    let mut stream = client.to_response_broadcast_stream();

    // Create a channel that can report when we should stop the loop based on a received request
    let (tx_stop, mut rx_stop) = oneshot::channel::<()>();

    // We also want to spawn a task to handle sending stdin to the remote process
    let mut rx = spawn_stdin_reader();
    tokio::spawn(async move {
        let mut buf = StringBuf::new();

        while let Some(data) = rx.recv().await {
            match config {
                // Special exit condition for interactive mode
                _ if buf.trim() == "exit" => {
                    if let Err(_) = tx_stop.send(()) {
                        error!("Failed to close interactive loop!");
                    }
                    break;
                }

                // For json mode, all stdin is treated as individual requests
                LoopConfig::Json => {
                    buf.push_str(&data);
                    let (lines, new_buf) = buf.into_full_lines();
                    buf = new_buf;

                    // For each complete line, parse it as json and
                    if let Some(lines) = lines {
                        for data in lines.lines() {
                            debug!("Client sending request: {:?}", data);
                            let result = serde_json::from_str(&data)
                                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x));
                            match result {
                                Ok(req) => match client.send(req).await {
                                    Ok(res) => match format_response(Mode::Json, res) {
                                        Ok(out) => out.print(),
                                        Err(x) => error!("Failed to format response: {}", x),
                                    },
                                    Err(x) => {
                                        error!("Failed to send request: {}", x)
                                    }
                                },
                                Err(x) => {
                                    error!("Failed to serialize request ('{}'): {}", data, x);
                                }
                            }
                        }
                    }
                }

                // For interactive shell mode, parse stdin as individual commands
                LoopConfig::Shell => {
                    buf.push_str(&data);
                    let (lines, new_buf) = buf.into_full_lines();
                    buf = new_buf;

                    if let Some(lines) = lines {
                        for data in lines.lines() {
                            trace!("Shell processing line: {:?}", data);
                            if data.trim().is_empty() {
                                continue;
                            }

                            debug!("Client sending command: {:?}", data);

                            // NOTE: We have to stick something in as the first argument as clap/structopt
                            //       expect the binary name as the first item in the iterator
                            let payload_result = RequestPayload::from_iter_safe(
                                std::iter::once("distant")
                                    .chain(data.trim().split(' ').filter(|s| !s.trim().is_empty())),
                            );
                            match payload_result {
                                Ok(payload) => {
                                    match client.send(Request::new(tenant.as_str(), payload)).await
                                    {
                                        Ok(res) => match format_response(Mode::Shell, res) {
                                            Ok(out) => out.print(),
                                            Err(x) => error!("Failed to format response: {}", x),
                                        },
                                        Err(x) => {
                                            error!("Failed to send request: {}", x)
                                        }
                                    }
                                }
                                Err(x) => {
                                    error!("Failed to parse command: {}", x);
                                }
                            }
                        }
                    }
                }

                // For non-interactive shell mode, all stdin is treated as a proc's stdin
                LoopConfig::Proc { id } => {
                    debug!("Client sending stdin: {:?}", data);
                    let req = Request::new(tenant.as_str(), RequestPayload::ProcStdin { id, data });
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
            let done = res.payload.is_proc_done() && config.is_proc();

            format_response(config.into(), res)?.print();

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

fn spawn_stdin_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(1);

    // NOTE: Using blocking I/O per tokio's advice to read from stdin line-by-line and then
    //       pass the results to a separate async handler to forward to the remote process
    std::thread::spawn(move || {
        use std::io::{self, BufReader, Read};
        let mut stdin = BufReader::new(io::stdin());

        // Maximum chunk that we expect to read at any one time
        let mut buf = [0; MAX_PIPE_CHUNK_SIZE];

        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    match String::from_utf8(buf[..n].to_vec()) {
                        Ok(text) => {
                            if let Err(x) = tx.blocking_send(text) {
                                error!(
                                    "Failed to pass along stdin to be sent to remote process: {}",
                                    x
                                );
                            }
                        }
                        Err(x) => {
                            error!("Input over stdin is invalid: {}", x);
                        }
                    }
                    std::thread::yield_now();
                }
            }
        }
    });

    rx
}

/// Represents the output content and destination
pub enum ResponseOut {
    Stdout(String),
    StdoutLine(String),
    Stderr(String),
    StderrLine(String),
    None,
}

impl ResponseOut {
    pub fn print(self) {
        match self {
            Self::Stdout(x) => print!("{}", x),
            Self::StdoutLine(x) => println!("{}", x),
            Self::Stderr(x) => eprint!("{}", x),
            Self::StderrLine(x) => eprintln!("{}", x),
            Self::None => {}
        }
    }
}

pub fn format_response(mode: Mode, res: Response) -> io::Result<ResponseOut> {
    Ok(match mode {
        Mode::Json => ResponseOut::StdoutLine(format!(
            "{}",
            serde_json::to_string(&res)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
        )),
        Mode::Shell => format_shell(res),
    })
}

fn format_shell(res: Response) -> ResponseOut {
    match res.payload {
        ResponsePayload::Ok => ResponseOut::None,
        ResponsePayload::Error { description } => {
            ResponseOut::StderrLine(format!("Failed: '{}'.", description))
        }
        ResponsePayload::Blob { data } => {
            ResponseOut::StdoutLine(String::from_utf8_lossy(&data).to_string())
        }
        ResponsePayload::Text { data } => ResponseOut::StdoutLine(data),
        ResponsePayload::DirEntries { entries, .. } => ResponseOut::StdoutLine(format!(
            "{}",
            entries
                .into_iter()
                .map(|entry| {
                    format!(
                        "{}{}",
                        entry.path.as_os_str().to_string_lossy(),
                        if entry.file_type.is_dir() {
                            // NOTE: This can be different from the server if
                            //       the server OS is unix and the client is
                            //       not or vice versa; for now, this doesn't
                            //       matter as we only support unix-based
                            //       operating systems, but something to keep
                            //       in mind
                            std::path::MAIN_SEPARATOR.to_string()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect::<Vec<String>>()
                .join("\n"),
        )),
        ResponsePayload::Metadata { data } => ResponseOut::StdoutLine(format!(
            concat!(
                "Type: {}\n",
                "Len: {}\n",
                "Readonly: {}\n",
                "Created: {}\n",
                "Last Accessed: {}\n",
                "Last Modified: {}",
            ),
            data.file_type.as_ref(),
            data.len,
            data.readonly,
            data.created.unwrap_or_default(),
            data.accessed.unwrap_or_default(),
            data.modified.unwrap_or_default(),
        )),
        ResponsePayload::ProcEntries { entries } => ResponseOut::StdoutLine(format!(
            "{}",
            entries
                .into_iter()
                .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
                .collect::<Vec<String>>()
                .join("\n"),
        )),
        ResponsePayload::ProcStart { .. } => ResponseOut::None,
        ResponsePayload::ProcStdout { data, .. } => ResponseOut::Stdout(data),
        ResponsePayload::ProcStderr { data, .. } => ResponseOut::Stderr(data),
        ResponsePayload::ProcDone { id, success, code } => {
            if success {
                ResponseOut::None
            } else if let Some(code) = code {
                ResponseOut::StderrLine(format!("Proc {} failed with code {}", id, code))
            } else {
                ResponseOut::StderrLine(format!("Proc {} failed", id))
            }
        }
    }
}
