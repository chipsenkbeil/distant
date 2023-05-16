use std::ops::{Deref, DerefMut};

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};

/// Represents some command with arguments to execute
#[derive(Clone, Debug, Display, From, Into, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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

#[cfg(feature = "schemars")]
impl Cmd {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Cmd)
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
