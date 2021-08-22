use crate::{
    cli::{buf::StringBuf, Format, ResponseOut},
    core::{
        client::Session,
        constants::MAX_PIPE_CHUNK_SIZE,
        data::{Request, Response},
        net::DataStream,
    },
};
use log::*;
use std::{
    io::{self, BufReader, Read},
    sync::Arc,
    thread,
};
use tokio::sync::{mpsc, watch};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

/// Represents a wrapper around a session that provides CLI functionality such as reading from
/// stdin and piping results back out to stdout
pub struct CliSession<T>
where
    T: DataStream,
{
    inner: Session<T>,
}

impl<T> CliSession<T>
where
    T: DataStream,
{
    pub fn new(inner: Session<T>) -> Self {
        Self { inner }
    }
}

// TODO TODO TODO:
//
// 1. Change watch to broadcast if going to use in both loops, otherwise just make
//    it an mpsc otherwise
// 2. Need to provide outgoing requests function with logic from inner.rs to create a request
//    based on the format (json or shell), where json uses serde_json::from_str and shell
//    uses Request::new(tenant.as_str(), vec![RequestData::from_iter_safe(...)])
// 3. Need to add a wait method to block on the running tasks
// 4. Need to add an abort method to abort the tasks
// 5. Is there any way to deal with the blocking thread for stdin to kill it? This isn't a big
//    deal as the shutdown would only be happening on client termination anyway, but still...

/// Helper function that loops, processing incoming responses not tied to a request to be sent out
/// over stdout/stderr
async fn process_incoming_responses(
    mut stream: BroadcastStream<Response>,
    format: Format,
    mut exit: watch::Receiver<bool>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            res = stream.next() => {
                match res {
                    Some(Ok(res)) => ResponseOut::new(format, res)?.print(),
                    Some(Err(x)) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, x)),
                    None => return Ok(()),
                }
            }
            _ = exit.changed() => {
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
            for line in lines.lines() {
                trace!("Processing line: {:?}", line);
                if line.trim().is_empty() {
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

/// Creates a new thread that performs stdin reads in a blocking fashion, returning
/// a handle to the thread and a receiver that will be sent input as it becomes available
fn spawn_stdin_reader() -> (thread::JoinHandle<()>, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(1);

    // NOTE: Using blocking I/O per tokio's advice to read from stdin line-by-line and then
    //       pass the results to a separate async handler to forward to the remote process
    let handle = thread::spawn(move || {
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
                    thread::yield_now();
                }
            }
        }
    });

    (handle, rx)
}
