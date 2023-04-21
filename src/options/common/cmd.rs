use clap::Args;
use std::fmt;
use std::str::FromStr;

/// Represents some command with arguments to execute.
///
/// NOTE: Must be derived with `#[clap(flatten)]` to properly take effect.
#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    /// The command to execute.
    #[clap(name = "CMD")]
    cmd: String,

    /// Arguments to provide to the command.
    #[clap(name = "ARGS")]
    args: Vec<String>,
}

impl Cmd {
    /// Creates a new command from the given `cmd`.
    pub fn new<C, I, A>(cmd: C, args: I) -> Self
    where
        C: Into<String>,
        I: Iterator<Item = A>,
        A: Into<String>,
    {
        Self {
            cmd: cmd.into(),
            args: args.map(Into::into).collect(),
        }
    }
}

impl From<Cmd> for String {
    fn from(cmd: Cmd) -> Self {
        cmd.to_string()
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cmd)?;
        for arg in self.args.iter() {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

impl<'a> From<&'a str> for Cmd {
    /// Parses `s` into [`Cmd`], or panics if unable to parse.
    fn from(s: &'a str) -> Self {
        s.parse().expect("Failed to parse into cmd")
    }
}

impl FromStr for Cmd {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tokens = if cfg!(unix) {
            shell_words::split(s)?
        } else if cfg!(windows) {
            winsplit::split(s)
        } else {
            unreachable!(
                "FromStr<Cmd>: Unsupported operating system outside Unix and Windows families!"
            );
        };

        // If we get nothing, then we want an empty command
        if tokens.is_empty() {
            return Ok(Self {
                cmd: String::new(),
                args: Vec::new(),
            });
        }

        let mut it = tokens.into_iter();
        Ok(Self {
            cmd: it.next().unwrap(),
            args: it.collect(),
        })
    }
}

/*
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
                .help("")
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
        Self::augment_args(cmd)
    }
} */

/* #[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cmd() {
        Cmd::augment_args(Command::new("distant")).debug_assert();
    }
} */
