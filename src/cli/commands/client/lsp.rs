use super::link::RemoteProcessLink;
use crate::cli::{CliError, CliResult};
use distant_core::{data::PtySize, DistantChannel, RemoteLspCommand};
use terminal_size::{terminal_size, Height, Width};

#[derive(Clone)]
pub struct Lsp(DistantChannel);

impl Lsp {
    pub fn new(channel: DistantChannel) -> Self {
        Self(channel)
    }

    pub async fn spawn(self, cmd: impl Into<String>, persist: bool, pty: bool) -> CliResult<()> {
        let mut proc = RemoteLspCommand::new()
            .persist(persist)
            .pty(if pty {
                terminal_size().map(|(Width(width), Height(height))| {
                    PtySize::from_rows_and_cols(height, width)
                })
            } else {
                None
            })
            .spawn(self.0, cmd)
            .await?;

        // Now, map the remote LSP server's stdin/stdout/stderr to our own process
        let link = RemoteProcessLink::from_remote_lsp_pipes(
            proc.stdin.take(),
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
        );

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
