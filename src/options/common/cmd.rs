use clap::error::{Error, ErrorKind};
use clap::{Arg, ArgAction, ArgMatches, Args, Command, FromArgMatches};
use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

/// Represents some command with arguments to execute
#[derive(Clone, Debug, Display, From, Into, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cmd(String);

impl Cmd {
    /// Creates a new command from the given `cmd`
    pub fn new(cmd: impl Into<String>) -> Self {
        Self(cmd.into())
    }

    /// Returns reference to the program portion of the command
    pub fn program(&self) -> &str {
        match self.0.split_once(' ') {
            Some((program, _)) => program.trim(),
            None => self.0.trim(),
        }
    }

    /// Returns reference to the arguments portion of the command
    pub fn arguments(&self) -> &str {
        match self.0.split_once(' ') {
            Some((_, arguments)) => arguments.trim(),
            None => "",
        }
    }
}

impl Deref for Cmd {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Cmd {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> From<&'a str> for Cmd {
    fn from(s: &'a str) -> Self {
        Self(s.to_string())
    }
}

impl FromArgMatches for Cmd {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, Error> {
        let mut matches = matches.clone();
        Self::from_arg_matches_mut(&mut matches)
    }
    fn from_arg_matches_mut(matches: &mut ArgMatches) -> Result<Self, Error> {
        let cmd = matches.get_one::<String>("cmd").ok_or_else(|| {
            Error::raw(
                ErrorKind::MissingRequiredArgument,
                "program must be specified",
            )
        })?;
        let args: Vec<String> = matches
            .get_many::<String>("arg")
            .unwrap_or_default()
            .map(ToString::to_string)
            .collect();
        Ok(Self::new(format!("{cmd} {}", args.join(" "))))
    }
    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), Error> {
        let mut matches = matches.clone();
        self.update_from_arg_matches_mut(&mut matches)
    }
    fn update_from_arg_matches_mut(&mut self, _matches: &mut ArgMatches) -> Result<(), Error> {
        Ok(())
    }
}

impl Args for Cmd {
    fn augment_args(cmd: Command) -> Command {
        cmd.arg(
            Arg::new("cmd")
                .required(true)
                .value_name("CMD")
                .action(ArgAction::Set),
        )
        .trailing_var_arg(true)
        .arg(
            Arg::new("arg")
                .value_name("ARGS")
                .num_args(1..)
                .action(ArgAction::Append),
        )
    }
    fn augment_args_for_update(cmd: Command) -> Command {
        cmd
    }
}
