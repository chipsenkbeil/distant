use super::link::RemoteProcessLink;
use crate::cli::{CliError, CliResult};
use distant_core::{
    data::{Environment, PtySize},
    DistantChannel, RemoteCommand,
};
use log::*;
use std::{io, time::Duration};
use terminal_size::{terminal_size, Height, Width};
use termwiz::{
    caps::Capabilities,
    input::{InputEvent, KeyCodeEncodeModes},
    terminal::{new_terminal, Terminal},
};

#[derive(Clone)]
pub struct Shell(DistantChannel);

impl Shell {
    pub fn new(channel: DistantChannel) -> Self {
        Self(channel)
    }

    pub async fn spawn(
        self,
        cmd: impl Into<Option<String>>,
        mut environment: Environment,
        persist: bool,
    ) -> CliResult<()> {
        // Automatically add TERM=xterm-256color if not specified
        if !environment.contains_key("TERM") {
            environment.insert("TERM".to_string(), "xterm-256color".to_string());
        }

        let mut proc = RemoteCommand::new()
            .persist(persist)
            .environment(environment)
            .pty(
                terminal_size()
                    .map(|(Width(cols), Height(rows))| PtySize::from_rows_and_cols(rows, cols)),
            )
            .spawn(self.0, cmd.into().unwrap_or_else(|| "/bin/sh".to_string()))
            .await?;

        // Create a new terminal in raw mode
        let mut terminal = new_terminal(
            Capabilities::new_from_env().map_err(|x| io::Error::new(io::ErrorKind::Other, x))?,
        )
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        terminal
            .set_raw_mode()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        let mut stdin = proc.stdin.take().unwrap();
        let resizer = proc.clone_resizer();
        tokio::spawn(async move {
            while let Ok(input) = terminal.poll_input(Some(Duration::new(0, 0))) {
                match input {
                    Some(InputEvent::Key(ev)) => {
                        if let Ok(input) = ev.key.encode(
                            ev.modifiers,
                            KeyCodeEncodeModes {
                                enable_csi_u_key_encoding: false,
                                application_cursor_keys: false,
                                newline_mode: false,
                            },
                        ) {
                            if let Err(x) = stdin.write_str(input).await {
                                error!("Failed to write to stdin of remote process: {}", x);
                                break;
                            }
                        }
                    }
                    Some(InputEvent::Resized { cols, rows }) => {
                        if let Err(x) = resizer
                            .resize(PtySize::from_rows_and_cols(rows as u16, cols as u16))
                            .await
                        {
                            error!("Failed to resize remote process: {}", x);
                            break;
                        }
                    }
                    Some(_) => continue,
                    None => tokio::time::sleep(Duration::from_millis(1)).await,
                }
            }
        });

        // Now, map the remote shell's stdout/stderr to our own process,
        // while stdin is handled by the task above
        let link = RemoteProcessLink::from_remote_pipes(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
        );

        // Continually loop to check for terminal resize changes while the process is still running
        let status = proc.wait().await?;

        // Shut down our link
        link.shutdown().await;

        if !status.success {
            if let Some(code) = status.code {
                return Err(CliError::from(code));
            } else {
                return Err(CliError::from(1));
            }
        }

        Ok(())
    }
}
