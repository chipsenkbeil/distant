use crate::server::process::{InputChannel, ProcessKiller, ProcessPty};

/// Holds information related to a spawned process on the server
pub struct ProcessState {
    pub cmd: String,
    pub args: Vec<String>,
    pub persist: bool,

    pub id: usize,
    pub stdin: Option<Box<dyn InputChannel>>,
    pub killer: Box<dyn ProcessKiller>,
    pub pty: Box<dyn ProcessPty>,
}
