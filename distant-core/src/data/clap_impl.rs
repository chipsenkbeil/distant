use crate::{DistantMsg, DistantRequestData};
use clap::{
    error::{Error, ErrorKind},
    ArgMatches, Command, FromArgMatches, Subcommand,
};

impl FromArgMatches for DistantMsg<DistantRequestData> {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, Error> {
        match matches.subcommand() {
            Some(("single", args)) => Ok(Self::Single(DistantRequestData::from_arg_matches(args)?)),
            Some((_, _)) => Err(Error::raw(
                ErrorKind::UnrecognizedSubcommand,
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
                    ErrorKind::UnrecognizedSubcommand,
                    "Valid subcommand is `single`",
                ))
            }
            None => (),
        };
        Ok(())
    }
}

impl Subcommand for DistantMsg<DistantRequestData> {
    fn augment_subcommands(cmd: Command<'_>) -> Command<'_> {
        cmd.subcommand(DistantRequestData::augment_subcommands(Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn augment_subcommands_for_update(cmd: Command<'_>) -> Command<'_> {
        cmd.subcommand(DistantRequestData::augment_subcommands(Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn has_subcommand(name: &str) -> bool {
        matches!(name, "single")
    }
}
