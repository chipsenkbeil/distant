use crate::Merge;
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    net::{AddrParseError, IpAddr},
    path::PathBuf,
    str::FromStr,
};

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindAddress {
    Ssh,
    Any,
    Ip(IpAddr),
}

impl FromStr for BindAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Ok(if s.eq_ignore_ascii_case("ssh") {
            Self::Ssh
        } else if s.eq_ignore_ascii_case("any") {
            Self::Any
        } else {
            s.parse()?
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_log_level_filter(self) -> log::LevelFilter {
        match self {
            Self::Off => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

/// Contains options that are common across subcommands
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommonConfig {
    /// Quiet mode, suppresses all logging (shortcut for log level off)
    #[clap(short, long, global = true)]
    pub quiet: bool,

    /// Log level to use throughout the application
    #[clap(long, global = true, case_insensitive = true, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Log output to disk instead of stderr
    #[clap(long, global = true)]
    pub log_file: Option<PathBuf>,

    /// Represents the maximum time (in seconds) to wait for a network request before timing out
    #[clap(short, long, global = true)]
    pub timeout: Option<f32>,
}

impl CommonConfig {
    pub fn log_level_or_default(&self) -> LogLevel {
        self.log_level.as_ref().copied().unwrap_or_default()
    }
}

impl Merge for CommonConfig {
    fn merge(&mut self, other: Self) {
        self.quiet = other.quiet;
        if let Some(x) = other.log_level {
            self.log_level = Some(x);
        }
        if let Some(x) = other.log_file {
            self.log_file = Some(x);
        }
        if let Some(x) = other.timeout {
            self.timeout = Some(x);
        }
    }
}
