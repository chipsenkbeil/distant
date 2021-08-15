use crate::{
    cli::opt::Mode,
    core::{
        constants::MAX_PIPE_CHUNK_SIZE,
        data::{Error, Request, RequestData, Response, ResponseData},
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
                            let result = RequestData::from_iter_safe(
                                std::iter::once("distant")
                                    .chain(data.trim().split(' ').filter(|s| !s.trim().is_empty())),
                            );
                            match result {
                                Ok(data) => {
                                    match client
                                        .send(Request::new(tenant.as_str(), vec![data]))
                                        .await
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
                    let req =
                        Request::new(tenant.as_str(), vec![RequestData::ProcStdin { id, data }]);
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

            // NOTE: If the loop is for a proxy process, we should assume that the payload
            //       is all-or-nothing for the done check
            let done = config.is_proc() && res.payload.iter().any(|x| x.is_proc_done());

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
            Self::Stdout(x) => {
                // NOTE: Because we are not including a newline in the output,
                //       it is not guaranteed to be written out. In the case of
                //       LSP protocol, the JSON content is not followed by a
                //       newline and was not picked up when the response was
                //       sent back to the client; so, we need to manually flush
                use std::io::Write;
                print!("{}", x);
                if let Err(x) = std::io::stdout().lock().flush() {
                    error!("Failed to flush stdout: {}", x);
                }
            }
            Self::StdoutLine(x) => println!("{}", x),
            Self::Stderr(x) => {
                use std::io::Write;
                eprint!("{}", x);
                if let Err(x) = std::io::stderr().lock().flush() {
                    error!("Failed to flush stderr: {}", x);
                }
            }
            Self::StderrLine(x) => eprintln!("{}", x),
            Self::None => {}
        }
    }
}

pub fn format_response(mode: Mode, res: Response) -> io::Result<ResponseOut> {
    let payload_cnt = res.payload.len();

    Ok(match mode {
        Mode::Json => ResponseOut::StdoutLine(format!(
            "{}",
            serde_json::to_string(&res)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
        )),

        // NOTE: For shell, we assume a singular entry in the response's payload
        Mode::Shell if payload_cnt != 1 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Got {} entries in payload data, but shell expects exactly 1",
                    payload_cnt
                ),
            ))
        }
        Mode::Shell => format_shell(res.payload.into_iter().next().unwrap()),
    })
}

fn format_shell(data: ResponseData) -> ResponseOut {
    match data {
        ResponseData::Ok => ResponseOut::None,
        ResponseData::Error(Error { kind, description }) => {
            ResponseOut::StderrLine(format!("Failed ({}): '{}'.", kind, description))
        }
        ResponseData::Blob { data } => {
            ResponseOut::StdoutLine(String::from_utf8_lossy(&data).to_string())
        }
        ResponseData::Text { data } => ResponseOut::StdoutLine(data),
        ResponseData::DirEntries { entries, .. } => ResponseOut::StdoutLine(format!(
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
        ResponseData::Exists(exists) => {
            if exists {
                ResponseOut::StdoutLine("Does exist.".to_string())
            } else {
                ResponseOut::StdoutLine("Does not exist.".to_string())
            }
        }
        ResponseData::Metadata {
            canonicalized_path,
            file_type,
            len,
            readonly,
            accessed,
            created,
            modified,
        } => ResponseOut::StdoutLine(format!(
            concat!(
                "{}",
                "Type: {}\n",
                "Len: {}\n",
                "Readonly: {}\n",
                "Created: {}\n",
                "Last Accessed: {}\n",
                "Last Modified: {}",
            ),
            canonicalized_path
                .map(|p| format!("Canonicalized Path: {:?}\n", p))
                .unwrap_or_default(),
            file_type.as_ref(),
            len,
            readonly,
            created.unwrap_or_default(),
            accessed.unwrap_or_default(),
            modified.unwrap_or_default(),
        )),
        ResponseData::ProcEntries { entries } => ResponseOut::StdoutLine(format!(
            "{}",
            entries
                .into_iter()
                .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
                .collect::<Vec<String>>()
                .join("\n"),
        )),
        ResponseData::ProcStart { .. } => ResponseOut::None,
        ResponseData::ProcStdout { data, .. } => ResponseOut::Stdout(data),
        ResponseData::ProcStderr { data, .. } => ResponseOut::Stderr(data),
        ResponseData::ProcDone { id, success, code } => {
            if success {
                ResponseOut::None
            } else if let Some(code) = code {
                ResponseOut::StderrLine(format!("Proc {} failed with code {}", id, code))
            } else {
                ResponseOut::StderrLine(format!("Proc {} failed", id))
            }
        }
        ResponseData::SystemInfo {
            family,
            os,
            arch,
            current_dir,
            main_separator,
        } => ResponseOut::StdoutLine(format!(
            concat!(
                "Family: {:?}\n",
                "Operating System: {:?}\n",
                "Arch: {:?}\n",
                "Cwd: {:?}\n",
                "Path Sep: {:?}",
            ),
            family, os, arch, current_dir, main_separator,
        )),
    }
}
