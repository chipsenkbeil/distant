use crate::{
    buf::StringBuf, constants::MAX_PIPE_CHUNK_SIZE, opt::Format, output::ResponseOut, stdin,
};
use distant_core::{Mailbox, Request, RequestData, Session};
use log::*;
use std::io;
use structopt::StructOpt;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// Represents a wrapper around a session that provides CLI functionality such as reading from
/// stdin and piping results back out to stdout
pub struct CliSession {
    req_task: JoinHandle<()>,
}

impl CliSession {
    /// Creates a new instance of a session for use in CLI interactions being fed input using
    /// the program's stdin
    pub fn new_for_stdin(tenant: String, session: Session, format: Format) -> Self {
        let (_stdin_thread, stdin_rx) = stdin::spawn_channel(MAX_PIPE_CHUNK_SIZE);

        Self::new(tenant, session, format, stdin_rx)
    }

    /// Creates a new instance of a session for use in CLI interactions being fed input using
    /// the provided receiver
    pub fn new(
        tenant: String,
        session: Session,
        format: Format,
        stdin_rx: mpsc::Receiver<Vec<u8>>,
    ) -> Self {
        let map_line = move |line: &str| match format {
            Format::Json => serde_json::from_str(line)
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
            process_outgoing_requests(session, stdin_rx, format, map_line).await
        });

        Self { req_task }
    }

    /// Wait for the cli session to terminate
    pub async fn wait(self) -> io::Result<()> {
        match self.req_task.await {
            Ok(res) => Ok(res),
            Err(x) => Err(io::Error::new(io::ErrorKind::BrokenPipe, x)),
        }
    }
}

/// Helper function that loops, processing incoming responses to a mailbox
async fn process_mailbox(mut mailbox: Mailbox, format: Format, exit: oneshot::Receiver<()>) {
    let inner = async move {
        while let Some(res) = mailbox.next().await {
            match ResponseOut::new(format, res) {
                Ok(out) => out.print(),
                Err(x) => {
                    error!("Repsonse out failed: {}", x);
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = inner => {}
        _ = exit => {}
    }
}

/// Helper function that loops, processing outgoing requests created from stdin, and printing out
/// responses
async fn process_outgoing_requests<F>(
    mut session: Session,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
    format: Format,
    map_line: F,
) where
    F: Fn(&str) -> io::Result<Request>,
{
    let mut buf = StringBuf::new();
    let mut mailbox_exits = Vec::new();

    while let Some(data) = stdin_rx.recv().await {
        // TODO: Should we support raw bytes? If so, we need to rewrite map_line to take Vec<u8>
        let data = match String::from_utf8(data) {
            Ok(data) => data,
            Err(x) => {
                error!("Bad stdin: {}", x);
                continue;
            }
        };

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
                }

                // TODO: We need to consolidate MsgReceiver and this logic as this only
                //       allows messages sent completely on a single line rather than
                //       MsgReceiver's ability to get a multi-line message
                match map_line(line) {
                    Ok(req) => match session.mail(req).await {
                        Ok(mut mailbox) => {
                            // Wait to get our first response before moving on to the next line
                            // of input
                            if let Some(res) = mailbox.next().await {
                                // Convert to response to output, and when successful launch
                                // a handler for continued responses to the same request
                                // such as with processes
                                match ResponseOut::new(format, res) {
                                    Ok(out) => {
                                        out.print();

                                        let (tx, rx) = oneshot::channel();
                                        mailbox_exits.push(tx);
                                        tokio::spawn(process_mailbox(mailbox, format, rx));
                                    }
                                    Err(x) => {
                                        error!("Map line response out failed: {}", x);
                                    }
                                }
                            }
                        }
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

    // Close out any dangling mailbox handlers
    for tx in mailbox_exits {
        let _ = tx.send(());
    }
}
