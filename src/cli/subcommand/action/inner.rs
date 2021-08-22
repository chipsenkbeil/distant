use crate::{
    cli::opt::Format,
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

impl From<LoopConfig> for Format {
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
                // Special exit condition for interactive format
                _ if buf.trim() == "exit" => {
                    if let Err(_) = tx_stop.send(()) {
                        error!("Failed to close interactive loop!");
                    }
                    break;
                }

                // For json format, all stdin is treated as individual requests
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
                                    Ok(res) => match format_response(Format::Json, res) {
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

                // For interactive shell format, parse stdin as individual commands
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
                                        Ok(res) => match format_response(Format::Shell, res) {
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

                // For non-interactive shell format, all stdin is treated as a proc's stdin
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
            let res = res.map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))?;

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
