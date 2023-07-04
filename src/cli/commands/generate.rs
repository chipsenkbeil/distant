use std::{fs, io};

use anyhow::Context;
use clap::CommandFactory;
use clap_complete::generate as clap_generate;

use crate::options::{Config, GenerateSubcommand};
use crate::{CliResult, Options};

pub fn run(cmd: GenerateSubcommand) -> CliResult {
    let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
    rt.block_on(async_run(cmd))
}

async fn async_run(cmd: GenerateSubcommand) -> CliResult {
    match cmd {
        GenerateSubcommand::Config { output } => match output {
            Some(path) => tokio::fs::write(path, Config::default_raw_str())
                .await
                .context("Failed to write default config to {path:?}")?,
            None => println!("{}", Config::default_raw_str()),
        },

        GenerateSubcommand::Completion { output, shell } => {
            let name = "distant";
            let mut cmd = Options::command();

            if let Some(path) = output {
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
