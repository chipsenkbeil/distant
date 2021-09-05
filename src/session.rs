use crate::{
    buf::StringBuf, constants::MAX_PIPE_CHUNK_SIZE, opt::Format, output::ResponseOut, stdin,
};
use distant_core::{DataStream, Request, RequestData, Response, Session};
use log::*;
use std::{io, thread};
use structopt::StructOpt;
use tokio::{sync::mpsc, task::JoinHandle};

/// Represents a wrapper around a session that provides CLI functionality such as reading from
/// stdin and piping results back out to stdout
pub struct CliSession {
    _stdin_thread: thread::JoinHandle<()>,
    req_task: JoinHandle<()>,
    res_task: JoinHandle<io::Result<()>>,
}

impl CliSession {
    pub fn new<T>(tenant: String, mut session: Session<T>, format: Format) -> Self
    where
        T: DataStream + 'static,
    {
        let (stdin_thread, stdin_rx) = stdin::spawn_channel(MAX_PIPE_CHUNK_SIZE);

        let (exit_tx, exit_rx) = mpsc::channel(1);
        let broadcast = session.broadcast.take().unwrap();
        let res_task =
            tokio::spawn(
                async move { process_incoming_responses(broadcast, format, exit_rx).await },
            );

        let map_line = move |line: &str| match format {
            Format::Json => serde_json::from_str(&line)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x)),
            Format::Shell => {
                let data = RequestData::from_iter_safe(
                    std::iter::once("distant")
                        .chain(line.trim().split(' ').filter(|s| !s.trim().is_empty())),
                )
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x));

                data.map(|x| Request::new(tenant.to_string(), vec![x]))
            }
        };
        let req_task = tokio::spawn(async move {
            process_outgoing_requests(session, stdin_rx, exit_tx, format, map_line).await
        });

        Self {
            _stdin_thread: stdin_thread,
            req_task,
            res_task,
        }
    }

    /// Wait for the cli session to terminate
    pub async fn wait(self) -> io::Result<()> {
        match tokio::try_join!(self.req_task, self.res_task) {
            Ok((_, res)) => res,
            Err(x) => Err(io::Error::new(io::ErrorKind::BrokenPipe, x)),
        }
    }
}

/// Helper function that loops, processing incoming responses not tied to a request to be sent out
/// over stdout/stderr
async fn process_incoming_responses(
    mut broadcast: mpsc::Receiver<Response>,
    format: Format,
    mut exit: mpsc::Receiver<()>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            res = broadcast.recv() => {
                match res {
                    Some(res) => ResponseOut::new(format, res)?.print(),
                    None => return Ok(()),
                }
            }
            _ = exit.recv() => {
                return Ok(());
            }
        }
    }
}

/// Helper function that loops, processing outgoing requests created from stdin, and printing out
/// responses
async fn process_outgoing_requests<T, F>(
    mut session: Session<T>,
    mut stdin_rx: mpsc::Receiver<String>,
    exit_tx: mpsc::Sender<()>,
    format: Format,
    map_line: F,
) where
    T: DataStream,
    F: Fn(&str) -> io::Result<Request>,
{
    let mut buf = StringBuf::new();

    while let Some(data) = stdin_rx.recv().await {
        // Update our buffer with the new data and split it into concrete lines and remainder
        buf.push_str(&data);
        let (lines, new_buf) = buf.into_full_lines();
        buf = new_buf;

        // For each complete line, parse into a request
        if let Some(lines) = lines {
            for line in lines.lines().map(|line| line.trim()) {
                trace!("Processing line: {:?}", line);
                if line.is_empty() {
                    continue;
                } else if line == "exit" {
                    debug!("Got exit request, so closing cli session");
                    stdin_rx.close();
                    if let Err(_) = exit_tx.send(()).await {
                        error!("Failed to close cli session");
                    }
                    continue;
                }

                match map_line(line) {
                    Ok(req) => match session.send(req).await {
                        Ok(res) => match ResponseOut::new(format, res) {
                            Ok(out) => out.print(),
                            Err(x) => error!("Failed to format response: {}", x),
                        },
                        Err(x) => {
                            error!("Failed to send request: {}", x)
                        }
                    },
                    Err(x) => {
                        error!("Failed to parse line: {}", x);
                    }
                }
            }
        }
    }
}
