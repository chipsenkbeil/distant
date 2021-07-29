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
    let is_proc_req = req.payload.is_proc_run() || req.payload.is_proc_connect();
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

    print_response(cmd.format, res)?;

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
            print_response(cmd.format, res)?;

            if done {
                break;
            }
        }
    }

    Ok(())
}

fn print_response(fmt: ResponseFormat, res: Response) -> io::Result<()> {
    // If we are not shell format or we are shell format and got stdout/stderr, we want
    // to print out the results
    let is_fmt_shell = fmt.is_shell();
    let is_type_stderr = res.payload.is_proc_stderr();
    let is_type_stdout = res.payload.is_proc_stdout();
    let do_print = !is_fmt_shell || is_type_stderr || is_type_stdout;

    let out = format_response(fmt, res)?;

    // Print out our response if flagged to do so
    if do_print {
        // If we are shell format and got stderr, write it to stderr without altering content
        if is_fmt_shell && is_type_stderr {
            eprint!("{}", out);

        // Else, if we are shell format and got stdout, write it to stdout without altering content
        } else if is_fmt_shell && is_type_stdout {
            print!("{}", out);

        // Otherwise, always go to stdout with traditional println
        } else {
            println!("{}", out);
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

fn format_response(fmt: ResponseFormat, res: Response) -> io::Result<String> {
    Ok(match fmt {
        ResponseFormat::Human => format_human(res),
        ResponseFormat::Json => serde_json::to_string(&res)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        ResponseFormat::Shell => format_shell(res),
    })
}

fn format_shell(res: Response) -> String {
    match res.payload {
        ResponsePayload::ProcStdout { data, .. } => String::from_utf8_lossy(&data).to_string(),
        ResponsePayload::ProcStderr { data, .. } => String::from_utf8_lossy(&data).to_string(),
        _ => String::new(),
    }
}

fn format_human(res: Response) -> String {
    match res.payload {
        ResponsePayload::Ok => "Done.".to_string(),
        ResponsePayload::Error { description } => format!("Failed: '{}'.", description),
        ResponsePayload::Blob { data } => String::from_utf8_lossy(&data).to_string(),
        ResponsePayload::Text { data } => data,
        ResponsePayload::DirEntries { entries } => entries
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
        ResponsePayload::ProcEntries { entries } => entries
            .into_iter()
            .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
            .collect::<Vec<String>>()
            .join("\n"),
        ResponsePayload::ProcStart { id } => format!("Proc({}): Started.", id),
        ResponsePayload::ProcStdout { id, data } => {
            format!("Stdout({}): '{}'.", id, String::from_utf8_lossy(&data))
        }
        ResponsePayload::ProcStderr { id, data } => {
            format!("Stderr({}): '{}'.", id, String::from_utf8_lossy(&data))
        }
        ResponsePayload::ProcDone { id, success, code } => {
            if success {
                format!("Proc({}): Done.", id)
            } else if let Some(code) = code {
                format!("Proc({}): Failed with code {}.", id, code)
            } else {
                format!("Proc({}): Failed.", id)
            }
        }
    }
}
