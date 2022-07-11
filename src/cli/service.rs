use std::io;

mod kind;
mod launchd;

pub use kind::ServiceKind;

/// Interface for a service
pub trait Service {
    fn install(&self, ctx: ServiceInstallCtx) -> io::Result<()>;
    fn uninstall(&self, ctx: ServiceUninstallCtx) -> io::Result<()>;
    fn start(&self, ctx: ServiceStartCtx) -> io::Result<()>;
    fn stop(&self, ctx: ServiceStopCtx) -> io::Result<()>;
}

impl dyn Service {
    /// Creates a new service targeting the specific service type
    pub fn target(kind: ServiceKind) -> Box<dyn Service> {
        match kind {
            #[cfg(target_os = "macos")]
            ServiceKind::Launchd => Box::new(launchd::LaunchdService),
            #[cfg(unix)]
            ServiceKind::OpenRc => todo!(),
            #[cfg(unix)]
            ServiceKind::Rc => todo!(),
            #[cfg(windows)]
            ServiceKind::Sc => todo!(),
            #[cfg(unix)]
            ServiceKind::Systemd => todo!(),
        }
    }
}

pub struct ServiceInstallCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: String,

    /// Whether or not this service is at the user-level
    pub user: bool,

    /// Arguments to use for the program, including the program itself
    ///
    /// E.g. `/usr/local/bin/distant`, `manager`, `listen`
    pub args: Vec<String>,
}

pub struct ServiceUninstallCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: String,

    /// Whether or not this service is at the user-level
    pub user: bool,
}

pub struct ServiceStartCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: String,
}

pub struct ServiceStopCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: String,
}
