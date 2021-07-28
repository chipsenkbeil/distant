use crate::{
    data::{Request, RequestPayload, Response, ResponsePayload},
    net::{Client, TransportError},
    opt::{CommonOpt, ExecuteFormat, ExecuteSubcommand},
    utils::{Session, SessionError},
};
use derive_more::{Display, Error, From};
use tokio::io;
use tokio_stream::StreamExt;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    SessionError(SessionError),
    TransportError(TransportError),
}

pub fn run(cmd: ExecuteSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ExecuteSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let session = Session::load().await?;
    let client = Client::connect(session).await?;

    let req = Request::from(cmd.operation);

    // Special conditions for continuing to process responses
    let is_proc_req = req.payload.is_proc_run() || req.payload.is_proc_connect();
    let not_detach = if let RequestPayload::ProcRun { detach, .. } = req.payload {
        !detach
    } else {
        false
    };

    let res = client.send(req).await?;
    print_response(cmd.format, res)?;

    // If we are executing a process and not detaching, we want to continue receiving
    // responses sent to us
    if is_proc_req && not_detach {
        let mut stream = client.to_response_stream();
        while let Some(res) = stream.next().await {
            print_response(cmd.format, res)?;
        }
    }

    Ok(())
}

fn print_response(fmt: ExecuteFormat, res: Response) -> io::Result<()> {
    // If we are not program format or we are program format and got stdout/stderr, we want
    // to print out the results
    let is_fmt_program = fmt.is_program();
    let is_type_stderr = res.payload.is_proc_stderr();
    let do_print = !is_fmt_program || is_type_stderr || res.payload.is_proc_stdout();

    let out = format_response(fmt, res)?;

    // Print out our response if flagged to do so
    if do_print {
        // If we are program format and got stderr, write it to stderr
        if is_fmt_program && is_type_stderr {
            eprintln!("{}", out);

        // Otherwise, always go to stdout
        } else {
            println!("{}", out);
        }
    }

    Ok(())
}

fn format_response(fmt: ExecuteFormat, res: Response) -> io::Result<String> {
    Ok(match fmt {
        ExecuteFormat::Json => serde_json::to_string(&res)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        ExecuteFormat::Program => format_program(res),
        ExecuteFormat::Shell => format_human(res),
    })
}

fn format_program(res: Response) -> String {
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
