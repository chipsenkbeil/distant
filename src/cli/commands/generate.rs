use std::{fs, io};

use anyhow::Context;
use clap::CommandFactory;
use clap_complete::generate as clap_generate;
use distant_core::net::common::{Request, Response};
use distant_core::{DistantMsg, DistantRequestData, DistantResponseData};

use crate::options::{Config, GenerateSubcommand};
use crate::{CliResult, Options};

pub fn run(cmd: GenerateSubcommand) -> CliResult {
    let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
    rt.block_on(async_run(cmd))
}

async fn async_run(cmd: GenerateSubcommand) -> CliResult {
    match cmd {
        GenerateSubcommand::Config { file } => tokio::fs::write(file, Config::default_raw_str())
            .await
            .context("Failed to write default config to {file:?}")?,

        GenerateSubcommand::Schema { file } => {
            let request_schema =
                serde_json::to_value(&Request::<DistantMsg<DistantRequestData>>::root_schema())
                    .context("Failed to serialize request schema")?;
            let response_schema =
                serde_json::to_value(&Response::<DistantMsg<DistantResponseData>>::root_schema())
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

        GenerateSubcommand::Completion { file, shell } => {
            let name = "distant";
            let mut cmd = Options::command();

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
