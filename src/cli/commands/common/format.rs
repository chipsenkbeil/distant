use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

use distant_core::net::common::Response;
use distant_core::protocol::{
    self, ChangeKind, Error, FileType, Metadata, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryPathMatch, SystemInfo,
};
use log::*;
use tabled::settings::object::Rows;
use tabled::settings::style::Style;
use tabled::settings::{Alignment, Disable, Modify};
use tabled::{Table, Tabled};

use crate::options::Format;

#[derive(Default)]
struct FormatterState {
    /// Last seen path during search
    pub last_searched_path: Option<PathBuf>,
}

pub struct Formatter {
    format: Format,
    state: FormatterState,
}

impl Formatter {
    /// Create a new output message for the given response based on the specified format
    pub fn new(format: Format) -> Self {
        Self {
            format,
            state: Default::default(),
        }
    }

    /// Creates a new [`Formatter`] using [`Format`] of `Format::Shell`
    pub fn shell() -> Self {
        Self::new(Format::Shell)
    }

    /// Consumes the output message, printing it based on its configuration
    pub fn print(&mut self, res: Response<protocol::Msg<protocol::Response>>) -> io::Result<()> {
        let output = match self.format {
            Format::Json => Output::StdoutLine(
                serde_json::to_vec(&res)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
            ),

            // NOTE: For shell, we assume a singular entry in the response's payload
            Format::Shell if res.payload.is_batch() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Shell does not support batch responses",
                ))
            }
            Format::Shell => format_shell(&mut self.state, res.payload.into_single().unwrap()),
        };

        match output {
            Output::Stdout(x) => {
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
            Output::StdoutLine(x) => {
                if let Err(x) = io::stdout().lock().write_all(&x) {
                    error!("Failed to write stdout: {}", x);
                }

                if let Err(x) = io::stdout().lock().write(b"\n") {
                    error!("Failed to write stdout newline: {}", x);
                }
            }
            Output::Stderr(x) => {
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
            Output::StderrLine(x) => {
                if let Err(x) = io::stderr().lock().write_all(&x) {
                    error!("Failed to write stderr: {}", x);
                }

                if let Err(x) = io::stderr().lock().write(b"\n") {
                    error!("Failed to write stderr newline: {}", x);
                }
            }
            Output::None => {}
        }

        Ok(())
    }
}

/// Represents the output content and destination
enum Output {
    Stdout(Vec<u8>),
    StdoutLine(Vec<u8>),
    Stderr(Vec<u8>),
    StderrLine(Vec<u8>),
    None,
}

fn format_shell(state: &mut FormatterState, data: protocol::Response) -> Output {
    match data {
        protocol::Response::Ok => Output::None,
        protocol::Response::Error(Error { description, .. }) => {
            Output::StderrLine(description.into_bytes())
        }
        protocol::Response::Blob { data } => Output::StdoutLine(data),
        protocol::Response::Text { data } => Output::StdoutLine(data.into_bytes()),
        protocol::Response::DirEntries { entries, .. } => {
            #[derive(Tabled)]
            struct EntryRow {
                ty: String,
                path: String,
            }

            let table = Table::new(entries.into_iter().map(|entry| EntryRow {
                ty: String::from(match entry.file_type {
                    FileType::Dir => "<DIR>",
                    FileType::File => "",
                    FileType::Symlink => "<SYMLINK>",
                }),
                path: entry.path.to_string_lossy().to_string(),
            }))
            .with(Style::blank())
            .with(Disable::row(Rows::new(..1)))
            .with(Modify::new(Rows::new(..)).with(Alignment::left()))
            .to_string()
            .into_bytes();

            Output::Stdout(table)
        }
        protocol::Response::Changed(change) => Output::StdoutLine(
            format!(
                "{} {}",
                match change.kind {
                    ChangeKind::Create => "(Created)",
                    ChangeKind::Delete => "(Removed)",
                    x if x.is_access() => "(Accessed)",
                    x if x.is_modify() => "(Modified)",
                    x if x.is_rename() => "(Renamed)",
                    _ => "(Affected)",
                },
                change.path.to_string_lossy()
            )
            .into_bytes(),
        ),
        protocol::Response::Exists { value: exists } => {
            if exists {
                Output::StdoutLine(b"true".to_vec())
            } else {
                Output::StdoutLine(b"false".to_vec())
            }
        }
        protocol::Response::Metadata(Metadata {
            canonicalized_path,
            file_type,
            len,
            readonly,
            accessed,
            created,
            modified,
            unix,
            windows,
        }) => Output::StdoutLine(
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
                    .map(|p| format!("Canonicalized Path: {p:?}\n"))
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
        protocol::Response::SearchStarted { id } => {
            Output::StdoutLine(format!("Query {id} started").into_bytes())
        }
        protocol::Response::SearchDone { .. } => Output::None,
        protocol::Response::SearchResults { matches, .. } => {
            let mut files: HashMap<_, Vec<String>> = HashMap::new();
            let mut is_targeting_paths = false;

            for m in matches {
                match m {
                    SearchQueryMatch::Path(SearchQueryPathMatch { path, .. }) => {
                        // Create the entry with no lines called out
                        files.entry(path).or_default();
                        is_targeting_paths = true;
                    }

                    SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path,
                        lines,
                        line_number,
                        ..
                    }) => {
                        let file_matches = files.entry(path).or_default();

                        file_matches.push(format!(
                            "{line_number}:{}",
                            lines.to_string_lossy().trim_end()
                        ));
                    }
                }
            }

            let mut output = String::new();
            for (path, lines) in files {
                use std::fmt::Write;

                // If we are seening a new path, print it out
                if state.last_searched_path.as_deref() != Some(path.as_path()) {
                    // If we have already seen some path before, we would have printed it, and
                    // we want to add a space between it and the current path, but only if we are
                    // printing out file content matches and not paths
                    if state.last_searched_path.is_some() && !is_targeting_paths {
                        writeln!(&mut output).unwrap();
                    }

                    writeln!(&mut output, "{}", path.to_string_lossy()).unwrap();
                }

                for line in lines {
                    writeln!(&mut output, "{line}").unwrap();
                }

                // Update our last seen path
                state.last_searched_path = Some(path);
            }

            if !output.is_empty() {
                Output::Stdout(output.into_bytes())
            } else {
                Output::None
            }
        }
        protocol::Response::ProcSpawned { .. } => Output::None,
        protocol::Response::ProcStdout { data, .. } => Output::Stdout(data),
        protocol::Response::ProcStderr { data, .. } => Output::Stderr(data),
        protocol::Response::ProcDone { id, success, code } => {
            if success {
                Output::None
            } else if let Some(code) = code {
                Output::StderrLine(format!("Proc {id} failed with code {code}").into_bytes())
            } else {
                Output::StderrLine(format!("Proc {id} failed").into_bytes())
            }
        }
        protocol::Response::SystemInfo(SystemInfo {
            family,
            os,
            arch,
            current_dir,
            main_separator,
            username,
            shell,
        }) => Output::StdoutLine(
            format!(
                concat!(
                    "Family: {:?}\n",
                    "Operating System: {:?}\n",
                    "Arch: {:?}\n",
                    "Cwd: {:?}\n",
                    "Path Sep: {:?}\n",
                    "Username: {:?}\n",
                    "Shell: {:?}"
                ),
                family, os, arch, current_dir, main_separator, username, shell
            )
            .into_bytes(),
        ),
        protocol::Response::Version(version) => {
            #[derive(Tabled)]
            struct EntryRow {
                kind: String,
                description: String,
            }

            let table = Table::new(
                version
                    .capabilities
                    .into_sorted_vec()
                    .into_iter()
                    .map(|cap| EntryRow {
                        kind: cap.kind,
                        description: cap.description,
                    }),
            )
            .with(Style::ascii())
            .with(Modify::new(Rows::new(..)).with(Alignment::left()))
            .to_string()
            .into_bytes();

            Output::StdoutLine(table)
        }
    }
}
