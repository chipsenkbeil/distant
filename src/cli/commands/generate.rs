use crate::{cli::Opt, config::GenerateConfig, CliResult};
use anyhow::Context;
use clap::{CommandFactory, Subcommand};
use clap_complete::{generate as clap_generate, Shell};
use distant_core::{
    net::common::{Request, Response},
    DistantMsg, DistantRequestData, DistantResponseData,
};
use std::{fs, io, path::PathBuf};

#[derive(Debug, Subcommand)]
pub enum GenerateSubcommand {
    /// Generate JSON schema for server request/response
    Schema {
        /// If specified, will output to the file at the given path instead of stdout
        #[clap(long)]
        file: Option<PathBuf>,
    },

    // Generate completion info for CLI
    Completion {
        /// If specified, will output to the file at the given path instead of stdout
        #[clap(long)]
        file: Option<PathBuf>,

        /// Specific shell to target for the generated output
        #[clap(value_enum, value_parser)]
        shell: Shell,
    },
}

impl GenerateSubcommand {
    pub fn run(self, _config: GenerateConfig) -> CliResult {
        let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
        rt.block_on(Self::async_run(self))
    }

    async fn async_run(self) -> CliResult {
        match self {
            Self::Schema { file } => {
                let request_schema =
                    serde_json::to_value(&Request::<DistantMsg<DistantRequestData>>::root_schema())
                        .context("Failed to serialize request schema")?;
                let response_schema = serde_json::to_value(&Response::<
                    DistantMsg<DistantResponseData>,
                >::root_schema())
                .context("Failed to serialize response schema")?;

                let schema = serde_json::json!({
                    "request": request_schema,
                    "response": response_schema,
                });

                if let Some(path) = file {
                    serde_json::to_writer_pretty(
                        &mut fs::OpenOptions::new()
                            .create(true)
                            .write(true)
                            .open(&path)
                            .with_context(|| format!("Failed to open {path:?}"))?,
                        &schema,
                    )
                    .context("Failed to write to {path:?}")?;
                } else {
                    serde_json::to_writer_pretty(&mut io::stdout(), &schema)
                        .context("Failed to print to stdout")?;
                }
            }

            Self::Completion { file, shell } => {
                let name = "distant";
                let mut cmd = Opt::command();

                if let Some(path) = file {
                    clap_generate(
                        shell,
                        &mut cmd,
                        name,
                        &mut fs::OpenOptions::new()
                            .create(true)
                            .write(true)
                            .open(&path)
                            .with_context(|| format!("Failed to open {path:?}"))?,
                    )
                } else {
                    clap_generate(shell, &mut cmd, name, &mut io::stdout())
                }
            }
        }

        Ok(())
    }
}
