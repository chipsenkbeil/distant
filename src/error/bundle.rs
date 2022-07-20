use std::{error::Error, fmt};

/// Represents a collection of associated errors
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorBundle {
    msg: String,
    errors: Vec<(String, String)>,
}

impl ErrorBundle {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            errors: Vec::new(),
        }
    }

    pub fn push(&mut self, label: impl Into<String>, error: impl Error) {
        self.errors.push((label.into(), error.to_string()));
    }

    pub fn error_cnt(&self) -> usize {
        self.errors.len()
    }
}

impl fmt::Display for ErrorBundle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)?;
        if !self.errors.is_empty() {
            for (label, msg) in self.errors.iter() {
                writeln!(f)?;
                if !label.is_empty() {
                    write!(f, "{label}: ")?;
                }
                write!(f, "{msg}")?;
            }
        }

        Ok(())
    }
}

impl Error for ErrorBundle {}
