use crate::{data::Cmd, DistantMsg, DistantRequestData};
use clap::{
    error::{Error, ErrorKind},
    Arg, ArgAction, ArgMatches, Args, Command, FromArgMatches, Subcommand,
};

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

impl FromArgMatches for DistantMsg<DistantRequestData> {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, Error> {
        match matches.subcommand() {
            Some(("single", args)) => Ok(Self::Single(DistantRequestData::from_arg_matches(args)?)),
            Some((_, _)) => Err(Error::raw(
                ErrorKind::InvalidSubcommand,
                "Valid subcommand is `single`",
            )),
            None => Err(Error::raw(
                ErrorKind::MissingSubcommand,
                "Valid subcommand is `single`",
            )),
        }
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), Error> {
        match matches.subcommand() {
            Some(("single", args)) => {
                *self = Self::Single(DistantRequestData::from_arg_matches(args)?)
            }
            Some((_, _)) => {
                return Err(Error::raw(
                    ErrorKind::InvalidSubcommand,
                    "Valid subcommand is `single`",
                ))
            }
            None => (),
        };
        Ok(())
    }
}

impl Subcommand for DistantMsg<DistantRequestData> {
    fn augment_subcommands(cmd: Command) -> Command {
        cmd.subcommand(DistantRequestData::augment_subcommands(Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn augment_subcommands_for_update(cmd: Command) -> Command {
        cmd.subcommand(DistantRequestData::augment_subcommands(Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn has_subcommand(name: &str) -> bool {
        matches!(name, "single")
    }
}
