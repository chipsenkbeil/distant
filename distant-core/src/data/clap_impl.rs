use crate::{DistantMsg, DistantRequestData};

impl clap::FromArgMatches for DistantMsg<DistantRequestData> {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::error::Error> {
        match matches.subcommand() {
            Some(("single", args)) => Ok(Self::Single(DistantRequestData::from_arg_matches(args)?)),
            Some((_, _)) => Err(clap::error::Error::raw(
                clap::error::ErrorKind::UnrecognizedSubcommand,
                "Valid subcommand is `single`",
            )),
            None => Err(clap::error::Error::raw(
                clap::error::ErrorKind::MissingSubcommand,
                "Valid subcommand is `single`",
            )),
        }
    }

    fn update_from_arg_matches(
        &mut self,
        matches: &clap::ArgMatches,
    ) -> Result<(), clap::error::Error> {
        match matches.subcommand() {
            Some(("single", args)) => {
                *self = Self::Single(DistantRequestData::from_arg_matches(args)?)
            }
            Some((_, _)) => {
                return Err(clap::error::Error::raw(
                    clap::error::ErrorKind::UnrecognizedSubcommand,
                    "Valid subcommand is `single`",
                ))
            }
            None => (),
        };
        Ok(())
    }
}

impl clap::Subcommand for DistantMsg<DistantRequestData> {
    fn augment_subcommands(cmd: clap::Command<'_>) -> clap::Command<'_> {
        cmd.subcommand(DistantRequestData::augment_subcommands(clap::Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn augment_subcommands_for_update(cmd: clap::Command<'_>) -> clap::Command<'_> {
        cmd.subcommand(DistantRequestData::augment_subcommands(clap::Command::new(
            "single",
        )))
        .subcommand_required(true)
    }

    fn has_subcommand(name: &str) -> bool {
        matches!(name, "single")
    }
}
