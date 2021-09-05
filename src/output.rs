use crate::opt::Format;
use distant_core::{data::Error, Response, ResponseData};
use log::*;
use std::io;

/// Represents the output content and destination
pub enum ResponseOut {
    Stdout(String),
    StdoutLine(String),
    Stderr(String),
    StderrLine(String),
    None,
}

impl ResponseOut {
    /// Create a new output message for the given response based on the specified format
    pub fn new(format: Format, res: Response) -> io::Result<ResponseOut> {
        let payload_cnt = res.payload.len();

        Ok(match format {
            Format::Json => ResponseOut::StdoutLine(
                serde_json::to_string(&res)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
            ),

            // NOTE: For shell, we assume a singular entry in the response's payload
            Format::Shell if payload_cnt != 1 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Got {} entries in payload data, but shell expects exactly 1",
                        payload_cnt
                    ),
                ))
            }
            Format::Shell => format_shell(res.payload.into_iter().next().unwrap()),
        })
    }

    /// Consumes the output message, printing it based on its configuration
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
        ResponseData::DirEntries { entries, .. } => ResponseOut::StdoutLine(
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
        ),
        ResponseData::Exists(exists) => {
            if exists {
                ResponseOut::StdoutLine("true".to_string())
            } else {
                ResponseOut::StdoutLine("false".to_string())
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
        ResponseData::ProcEntries { entries } => ResponseOut::StdoutLine(
            entries
                .into_iter()
                .map(|entry| format!("{}: {} {}", entry.id, entry.cmd, entry.args.join(" ")))
                .collect::<Vec<String>>()
                .join("\n"),
        ),
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
