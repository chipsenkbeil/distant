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
