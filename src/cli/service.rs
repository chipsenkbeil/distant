use std::{fmt, io, str::FromStr};

mod kind;

#[cfg(target_os = "macos")]
mod launchd;

#[cfg(unix)]
mod openrc;

pub use kind::ServiceKind;

#[cfg(target_os = "macos")]
pub use launchd::LaunchdService;

#[cfg(unix)]
pub use openrc::OpenRcService;

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
    /// Creates a new service using the specified type, falling back to selecting
    /// based on native targeting for the current operating system if no type provided
    pub fn target_or_native(kind: impl Into<Option<ServiceKind>>) -> io::Result<Box<dyn Service>> {
        match kind.into() {
            Some(kind) => Ok(<dyn Service>::target(kind)),
            None => <dyn Service>::native_target(),
        }
    }

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

    /// Attempts to select a native target for the current operating system
    ///
    /// * For MacOS, this will use [`LaunchdService`]
    /// * For Windows, this will use [`ScService`]
    /// * For BSD variants, this will use [`RcService`]
    /// * For Linux variants, this will use either [`SystemdService`] or [`OpenRc`]
    pub fn native_target() -> io::Result<Box<dyn Service>> {
        #[cfg(target_os = "macos")]
        fn native_target_kind() -> io::Result<ServiceKind> {
            Ok(ServiceKind::Launchd)
        }

        #[cfg(target_os = "windows")]
        fn native_target_kind() -> io::Result<ServiceKind> {
            Ok(ServiceKind::Sc)
        }

        #[cfg(any(
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        fn native_target_kind() -> io::Result<ServiceKind> {
            Ok(ServiceKind::Rc)
        }

        #[cfg(target_os = "linux")]
        fn native_target_kind() -> io::Result<ServiceKind> {
            let service = <dyn Service>::target(ServiceKind::Systemd);
            if let Ok(true) = service.available() {
                return Ok(ServiceKind::Systemd);
            }

            let service = <dyn Service>::target(ServiceKind::OpenRc);
            if let Ok(true) = service.available() {
                return Ok(ServiceKind::OpenRc);
            }

            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Only systemd and openrc are supported on Linux",
            ))
        }

        Ok(Self::target(native_target_kind()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceLabel {
    pub qualifier: String,
    pub organization: String,
    pub application: String,
}

impl ServiceLabel {
    /// Produces a fully qualified name in the form of `{qualifier}.{organization}.{application}`
    pub fn to_qualified_name(&self) -> String {
        format!(
            "{}.{}.{}",
            self.qualifier, self.organization, self.application
        )
    }

    /// Produces a script name using the organization and application
    /// in the form of `{organization}-{application}`
    pub fn to_script_name(&self) -> String {
        format!("{}-{}", self.organization, self.application)
    }
}

impl fmt::Display for ServiceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}",
            self.qualifier, self.organization, self.application
        )
    }
}

impl FromStr for ServiceLabel {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tokens = s.split('.').collect::<Vec<&str>>();
        if tokens.len() != 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                concat!(
                    "Unexpected token count! ",
                    "Expected 3 items in the form {qualifier}.{organization}.{application}"
                ),
            ));
        }

        Ok(Self {
            qualifier: tokens[0].to_string(),
            organization: tokens[1].to_string(),
            application: tokens[2].to_string(),
        })
    }
}

pub struct ServiceInstallCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: ServiceLabel,

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
    pub label: ServiceLabel,

    /// Whether or not this service is at the user-level
    pub user: bool,
}

pub struct ServiceStartCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: ServiceLabel,
}

pub struct ServiceStopCtx {
    /// Label associated with the service
    ///
    /// E.g. `rocks.distant.manager`
    pub label: ServiceLabel,
}
