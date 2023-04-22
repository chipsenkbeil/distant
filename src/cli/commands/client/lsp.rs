use super::super::common::RemoteProcessLink;
use super::{CliError, CliResult};
use anyhow::Context;
use distant_core::{data::PtySize, DistantChannel, RemoteLspCommand};
use std::path::PathBuf;
use terminal_size::{terminal_size, Height, Width};

#[derive(Clone)]
pub struct Lsp(DistantChannel);

impl Lsp {
    pub fn new(channel: DistantChannel) -> Self {
        Self(channel)
    }

    pub async fn spawn(
        self,
        cmd: impl Into<String>,
        current_dir: Option<PathBuf>,
        pty: bool,
        max_chunk_size: usize,
    ) -> CliResult {
        let cmd = cmd.into();
        let mut proc = RemoteLspCommand::new()
            .pty(if pty {
                terminal_size().map(|(Width(width), Height(height))| {
                    PtySize::from_rows_and_cols(height, width)
                })
            } else {
                None
            })
            .current_dir(current_dir)
            .spawn(self.0, &cmd)
            .await
            .with_context(|| format!("Failed to spawn {cmd}"))?;

        // Now, map the remote LSP server's stdin/stdout/stderr to our own process
        let link = RemoteProcessLink::from_remote_lsp_pipes(
            proc.stdin.take(),
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
        );

        let status = proc.wait().await.context("Failed to wait for process")?;

        // Shut down our link
        link.shutdown().await;

        if !status.success {
            if let Some(code) = status.code {
                return Err(CliError::Exit(code as u8));
            } else {
                return Err(CliError::FAILURE);
            }
        }

        Ok(())
    }
}
