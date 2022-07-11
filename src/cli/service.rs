use std::io;

mod kind;
mod launchd;
mod openrc;

pub use kind::ServiceKind;

/// Interface for a service
pub trait Service {
    /// Determines if the service exists (e.g. is `launchd` available on the system?) and
    /// can be used
    fn available(&self) -> io::Result<bool>;

    /// Installs a new service
    fn install(&self, ctx: ServiceInstallCtx) -> io::Result<()>;

    /// Uninstalls an existing service
    fn uninstall(&self, ctx: ServiceUninstallCtx) -> io::Result<()>;

    /// Starts a service
    fn start(&self, ctx: ServiceStartCtx) -> io::Result<()>;

    /// Stops a running service
    fn stop(&self, ctx: ServiceStopCtx) -> io::Result<()>;
}

impl dyn Service {
    /// Creates a new service targeting the specific service type
    pub fn target(kind: ServiceKind) -> Box<dyn Service> {
        match kind {
            #[cfg(target_os = "macos")]
            ServiceKind::Launchd => Box::new(launchd::LaunchdService),
            #[cfg(unix)]
            ServiceKind::OpenRc => Box::new(openrc::OpenRcService),
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

    /// Path to the program to run
    ///
    /// E.g. `/usr/local/bin/distant`
    pub program: String,

    /// Arguments to use for the program
    ///
    /// E.g. `manager`, `listen`
    pub args: Vec<String>,
}

impl ServiceInstallCtx {
    /// Iterator over the program and its arguments
    pub fn cmd_iter(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.program.as_str()).chain(self.args_iter())
    }

    /// Iterator over the program arguments
    pub fn args_iter(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(String::as_str)
    }
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
