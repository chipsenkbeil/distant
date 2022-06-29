use crate::config::ReplFormat;
use distant_core::{
    data::{ChangeKind, DistantMsg, DistantResponseData, Error, Metadata, SystemInfo},
    net::Response,
};
use log::*;
use std::io;
use std::io::Write;

/// Represents the output content and destination
pub enum ResponseOut {
    Stdout(Vec<u8>),
    StdoutLine(Vec<u8>),
    Stderr(Vec<u8>),
    StderrLine(Vec<u8>),
    None,
}

impl ResponseOut {
    /// Create a new output message for the given response based on the specified format
    pub fn new(
        format: ReplFormat,
        res: Response<DistantMsg<DistantResponseData>>,
    ) -> io::Result<ResponseOut> {
        Ok(match format {
            ReplFormat::Json => ResponseOut::StdoutLine(
                serde_json::to_vec(&res)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
            ),

            // NOTE: For shell, we assume a singular entry in the response's payload
            ReplFormat::Shell if res.payload.is_batch() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Shell does not support batch responses",
                ))
            }
            ReplFormat::Shell => format_shell(res.payload.into_single().unwrap()),
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
                if let Err(x) = io::stdout().lock().write_all(&x) {
                    error!("Failed to write stdout: {}", x);
                }

                if let Err(x) = io::stdout().lock().flush() {
                    error!("Failed to flush stdout: {}", x);
                }
            }
            Self::StdoutLine(x) => {
                if let Err(x) = io::stdout().lock().write_all(&x) {
                    error!("Failed to write stdout: {}", x);
                }

                if let Err(x) = io::stdout().lock().write(b"\n") {
                    error!("Failed to write stdout newline: {}", x);
                }
            }
            Self::Stderr(x) => {
                // NOTE: Because we are not including a newline in the output,
                //       it is not guaranteed to be written out. In the case of
                //       LSP protocol, the JSON content is not followed by a
                //       newline and was not picked up when the response was
                //       sent back to the client; so, we need to manually flush
                if let Err(x) = io::stderr().lock().write_all(&x) {
                    error!("Failed to write stderr: {}", x);
                }

                if let Err(x) = io::stderr().lock().flush() {
                    error!("Failed to flush stderr: {}", x);
                }
            }
            Self::StderrLine(x) => {
                if let Err(x) = io::stderr().lock().write_all(&x) {
                    error!("Failed to write stderr: {}", x);
                }

                if let Err(x) = io::stderr().lock().write(b"\n") {
                    error!("Failed to write stderr newline: {}", x);
                }
            }
            Self::None => {}
        }
    }
}

fn format_shell(data: DistantResponseData) -> ResponseOut {
    match data {
        DistantResponseData::Ok => ResponseOut::None,
        DistantResponseData::Error(Error { kind, description }) => {
            ResponseOut::StderrLine(format!("Failed ({}): '{}'.", kind, description).into_bytes())
        }
        DistantResponseData::Blob { data } => ResponseOut::StdoutLine(data),
        DistantResponseData::Text { data } => ResponseOut::StdoutLine(data.into_bytes()),
        DistantResponseData::DirEntries { entries, .. } => ResponseOut::StdoutLine(
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
                .join("\n")
                .into_bytes(),
        ),
        DistantResponseData::Changed(change) => ResponseOut::StdoutLine(
            format!(
                "{}{}",
                match change.kind {
                    ChangeKind::Create => "Following paths were created:\n",
                    ChangeKind::Remove => "Following paths were removed:\n",
                    x if x.is_access_kind() => "Following paths were accessed:\n",
                    x if x.is_modify_kind() => "Following paths were modified:\n",
                    x if x.is_rename_kind() => "Following paths were renamed:\n",
                    _ => "Following paths were affected:\n",
                },
                change
                    .paths
                    .into_iter()
                    .map(|p| format!("* {}", p.to_string_lossy()))
                    .collect::<Vec<String>>()
                    .join("\n")
            )
            .into_bytes(),
        ),
        DistantResponseData::Exists { value: exists } => {
            if exists {
                ResponseOut::StdoutLine(b"true".to_vec())
            } else {
                ResponseOut::StdoutLine(b"false".to_vec())
            }
        }
        DistantResponseData::Metadata(Metadata {
            canonicalized_path,
            file_type,
            len,
            readonly,
            accessed,
            created,
            modified,
            unix,
            windows,
        }) => ResponseOut::StdoutLine(
            format!(
                concat!(
                    "{}",
                    "Type: {}\n",
                    "Len: {}\n",
                    "Readonly: {}\n",
                    "Created: {}\n",
                    "Last Accessed: {}\n",
                    "Last Modified: {}\n",
                    "{}",
                    "{}",
                    "{}",
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
                unix.map(|u| format!(
                    concat!(
                        "Owner Read: {}\n",
                        "Owner Write: {}\n",
                        "Owner Exec: {}\n",
                        "Group Read: {}\n",
                        "Group Write: {}\n",
                        "Group Exec: {}\n",
                        "Other Read: {}\n",
                        "Other Write: {}\n",
                        "Other Exec: {}",
                    ),
                    u.owner_read,
                    u.owner_write,
                    u.owner_exec,
                    u.group_read,
                    u.group_write,
                    u.group_exec,
                    u.other_read,
                    u.other_write,
                    u.other_exec
                ))
                .unwrap_or_default(),
                windows
                    .map(|w| format!(
                        concat!(
                            "Archive: {}\n",
                            "Compressed: {}\n",
                            "Encrypted: {}\n",
                            "Hidden: {}\n",
                            "Integrity Stream: {}\n",
                            "Normal: {}\n",
                            "Not Content Indexed: {}\n",
                            "No Scrub Data: {}\n",
                            "Offline: {}\n",
                            "Recall on Data Access: {}\n",
                            "Recall on Open: {}\n",
                            "Reparse Point: {}\n",
                            "Sparse File: {}\n",
                            "System: {}\n",
                            "Temporary: {}",
                        ),
                        w.archive,
                        w.compressed,
                        w.encrypted,
                        w.hidden,
                        w.integrity_stream,
                        w.normal,
                        w.not_content_indexed,
                        w.no_scrub_data,
                        w.offline,
                        w.recall_on_data_access,
                        w.recall_on_open,
                        w.reparse_point,
                        w.sparse_file,
                        w.system,
                        w.temporary,
                    ))
                    .unwrap_or_default(),
                if unix.is_none() && windows.is_none() {
                    String::from("\n")
                } else {
                    String::new()
                }
            )
            .into_bytes(),
        ),
        DistantResponseData::ProcSpawned { .. } => ResponseOut::None,
        DistantResponseData::ProcStdout { data, .. } => ResponseOut::Stdout(data),
        DistantResponseData::ProcStderr { data, .. } => ResponseOut::Stderr(data),
        DistantResponseData::ProcDone { id, success, code } => {
            if success {
                ResponseOut::None
            } else if let Some(code) = code {
                ResponseOut::StderrLine(
                    format!("Proc {} failed with code {}", id, code).into_bytes(),
                )
            } else {
                ResponseOut::StderrLine(format!("Proc {} failed", id).into_bytes())
            }
        }
        DistantResponseData::SystemInfo(SystemInfo {
            family,
            os,
            arch,
            current_dir,
            main_separator,
        }) => ResponseOut::StdoutLine(
            format!(
                concat!(
                    "Family: {:?}\n",
                    "Operating System: {:?}\n",
                    "Arch: {:?}\n",
                    "Cwd: {:?}\n",
                    "Path Sep: {:?}",
                ),
                family, os, arch, current_dir, main_separator,
            )
            .into_bytes(),
        ),
    }
}
