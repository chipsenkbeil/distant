use crate::{
    data::{Request, RequestPayload, Response, ResponsePayload},
    net::{Client, TransportError},
    opt::{CommonOpt, ResponseFormat, SendSubcommand},
    utils::{Session, SessionError},
};
use derive_more::{Display, Error, From};
use log::*;
use tokio::{io, sync::mpsc};
use tokio_stream::StreamExt;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    SessionError(SessionError),
    TransportError(TransportError),
}

pub fn run(cmd: SendSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: SendSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let session = Session::load().await?;
    let mut client = Client::connect(session).await?;

    let req = Request::from(cmd.operation);

    // Special conditions for continuing to process responses
    let is_proc_req = req.payload.is_proc_run();
    let not_detach = if let RequestPayload::ProcRun { detach, .. } = req.payload {
        !detach
    } else {
        false
    };

    let res = client.send(req).await?;

    // Store the spawned process id for using in sending stdin (if we spawned a proc)
    let proc_id = match &res.payload {
        ResponsePayload::ProcStart { id } => *id,
        _ => 0,
    };

    format_response(cmd.format, res)?.print();

    // If we are executing a process and not detaching, we want to continue receiving
    // responses sent to us
    if is_proc_req && not_detach {
        let mut stream = client.to_response_stream();

        // We also want to spawn a task to handle sending stdin to the remote process
        let mut rx = spawn_stdin_reader();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                trace!("Client sending stdin: {:?}", line);
                let req = Request::from(RequestPayload::ProcStdin {
                    id: proc_id,
                    data: line.into_bytes(),
                });
                let result = client.send(req).await;

                if let Err(x) = result {
                    error!(
                        "Failed to send stdin to remote process ({}): {}",
                        proc_id, x
                    );
                }
            }
        });

        while let Some(res) = stream.next().await {
            let res = res.map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Response stream no longer available",
                )
            })?;
            let done = res.payload.is_proc_done();

            format_response(cmd.format, res)?.print();

            if done {
                break;
            }
        }
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

fn format_response(fmt: ResponseFormat, res: Response) -> io::Result<ResponseOut> {
    Ok(match fmt {
        ResponseFormat::Json => ResponseOut::Stdout(
            serde_json::to_string(&res)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        ),
        ResponseFormat::Shell => format_shell(res),
    })
}

fn format_shell(res: Response) -> ResponseOut {
    match res.payload {
        ResponsePayload::Ok => ResponseOut::None,
        ResponsePayload::Error { description } => {
            ResponseOut::Stderr(format!("Failed: '{}'.", description))
        }
        ResponsePayload::Blob { data } => {
            ResponseOut::Stdout(String::from_utf8_lossy(&data).to_string())
        }
        ResponsePayload::Text { data } => ResponseOut::Stdout(data),
        ResponsePayload::DirEntries { entries } => ResponseOut::Stdout(
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
        ),
        ResponsePayload::ProcEntries { entries } => ResponseOut::Stdout(
            entries
                .into_iter()
                .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
                .collect::<Vec<String>>()
                .join("\n"),
        ),
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
                ResponseOut::Stderr(format!("Proc {} failed with code {}", id, code))
            } else {
                ResponseOut::Stderr(format!("Proc {} failed", id))
            }
        }
    }
}
