use tokio::{io, sync::mpsc};

/// Exit status of a remote process
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub success: bool,
    pub code: Option<i32>,
}

impl ExitStatus {
    /// Produces a new exit status representing a killed process
    pub fn killed() -> Self {
        Self {
            success: false,
            code: None,
        }
    }
}

impl<T, E> From<Result<T, E>> for ExitStatus
where
    T: Into<ExitStatus>,
    E: Into<ExitStatus>,
{
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(x) => x.into(),
            Err(x) => x.into(),
        }
    }
}

impl From<io::Error> for ExitStatus {
    fn from(err: io::Error) -> Self {
        Self {
            success: false,
            code: err.raw_os_error(),
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(status: std::process::ExitStatus) -> Self {
        Self {
            success: status.success(),
            code: status.code(),
        }
    }
}

/// Creates a new channel for when the exit status will be ready
pub fn channel() -> (WaitTx, WaitRx) {
    let (tx, rx) = mpsc::channel(1);
    (WaitTx::Pending(tx), WaitRx::Pending(rx))
}

/// Represents a notifier for a specific waiting state
#[derive(Debug)]
pub enum WaitTx {
    /// Notification has been sent
    Done,

    /// Notification has not been sent
    Pending(mpsc::Sender<ExitStatus>),
}

impl WaitTx {
    /// Returns true if exit status has been sent through channel
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }

    /// Send exit status to receiving-side of wait
    pub fn send<S>(&mut self, status: S) -> io::Result<()>
    where
        S: Into<ExitStatus>,
    {
        let status = status.into();

        match self {
            Self::Done => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Notifier is closed",
            )),
            Self::Pending(tx) => {
                let res = tx.blocking_send(status);
                *self = Self::Done;

                match res {
                    Ok(_) => Ok(()),
                    Err(x) => Err(io::Error::new(io::ErrorKind::Other, x)),
                }
            }
        }
    }

    /// Mark wait as completed using killed status
    pub fn kill(&mut self) -> io::Result<()> {
        self.send(ExitStatus::killed())
    }
}

/// Represents the state of waiting for an exit status
#[derive(Debug)]
pub enum WaitRx {
    /// Exit status is ready
    Ready(ExitStatus),

    /// If receiver for an exit status has been dropped without receiving the status
    Dropped,

    /// Exit status is not ready and has a "oneshot" to be invoked when available
    Pending(mpsc::Receiver<ExitStatus>),
}

impl WaitRx {
    /// Returns true if receiver of exit status is still waiting
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }

    /// Converts into an option, returning Some(status) if ready, otherwise None
    ///
    /// Note that this does NOT attempt to resolve a pending instance. To do that,
    /// this requires a mutation and should instead invoke `resolve`.
    pub fn to_option(&self) -> Option<ExitStatus> {
        match self {
            Self::Ready(status) => Some(*status),
            Self::Dropped => None,
            Self::Pending(_) => None,
        }
    }

    /// Waits until the exit status is resolved; can be called repeatedly after being
    /// resolved to immediately return the exit status again
    pub async fn recv(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Ready(status) => Ok(*status),
            Self::Dropped => Err(io::Error::new(
                io::ErrorKind::Other,
                "Internal resolver dropped",
            )),
            Self::Pending(rx) => match rx.recv().await {
                Some(status) => {
                    *self = Self::Ready(status);
                    Ok(status)
                }
                None => {
                    *self = Self::Dropped;
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Internal resolver dropped",
                    ))
                }
            },
        }
    }
}
