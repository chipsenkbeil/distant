use crate::{
    data::{Request, Response, ResponsePayload},
    net::{Client, TransportError},
    opt::{CommonOpt, ExecuteFormat, ExecuteSubcommand},
    utils::{Session, SessionError},
};
use derive_more::{Display, Error, From};
use tokio::io;

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

    let res = client.send(req).await?;
    let res_string = match cmd.format {
        ExecuteFormat::Json => serde_json::to_string(&res)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        ExecuteFormat::Shell => format_human(res),
    };
    println!("{}", res_string);

    // TODO: Process result to determine if we want to create a watch stream and continue
    //       to examine results

    Ok(())
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
        ResponsePayload::ProcList { entries } => entries
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
