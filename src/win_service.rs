use super::{ExitCode, ExitCodeError};
use log::*;
use std::{ffi::OsString, io, sync::mpsc, thread, time::Duration};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "distant_manager";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    pub args: Vec<std::ffi::OsString>,
}

impl Config {
    pub fn save(&self) -> io::Result<()> {
        let mut bytes = Vec::new();
        serde_json::to_writer(&mut bytes, self)
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        std::fs::write(Self::config_file(), bytes)
    }

    pub fn load() -> io::Result<Self> {
        let bytes = std::fs::read(Self::config_file())?;
        serde_json::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::Other, x))
    }

    pub fn delete() -> io::Result<()> {
        std::fs::remove_file(Self::config_file())
    }

    /// Stored next to the service exe
    fn config_file() -> std::path::PathBuf {
        let mut path = std::env::current_exe().unwrap();
        path.set_extension("exe.config");
        path
    }
}

pub enum ServiceError {
    /// Encountered when attempting to save the service config before starting the service
    FailedToCreateServiceConfig(io::Error),

    /// Encountered when attempting to delete the service config after service completes
    FailedToDeleteServiceConfig(io::Error),

    /// Encountered when starting the service, which can mean that we aren't running as a service
    Service(windows_service::Error),
}

pub fn run() -> Result<(), ServiceError> {
    // Save our CLI arguments to pass on to the service
    let config = Config {
        args: std::env::args_os().collect(),
    };
    config
        .save()
        .map_err(ServiceError::FailedToCreateServiceConfig)?;

    // Attempt to run as a service, deleting our config when completed
    // regardless of success
    let result = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    let config_result = config
        .delete()
        .map_err(ServiceError::FailedToDeleteServiceConfig);

    // Swallow the config error if we have a service error, otherwise display
    // the config error
    match (result, config_result) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(x), _) => Err(ServiceError::Service(x)),
        (_, Err(x)) => Err(x),
    }
}

/// Returns true if running as a windows service
pub fn is_windows_service() -> bool {
    use sysinfo::{Pid, PidExt, Process, ProcessExt, System, SystemExt};

    let mut system = System::new();

    // Get our own process pid
    let pid = Pid::from_u32(std::process::id());

    // Update our system's knowledge about our process
    system.refresh_process(pid);

    // Get our parent process' pid and update sustem's knowledge about parent process
    let maybe_parent_pid = system.process(pid).and_then(Process::parent);
    if let Some(pid) = maybe_parent_pid {
        system.refresh_process(pid);
    }

    // Check modeled after https://github.com/dotnet/extensions/blob/9069ee83c6ff1e4471cfbc07215c715c5ce157e1/src/Hosting/WindowsServices/src/WindowsServiceHelpers.cs#L31
    maybe_parent_pid
        .and_then(|pid| system.process(pid))
        .map(Process::exe)
        .and_then(Path::file_name)
        .map(OsStr::to_string_lossy)
        .map(|s| s.eq_ignore_ascii_case("services"))
        .unwrap_or_default()
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    if let Err(_e) = run_service() {
        // Handle the error, by logging or something.
    }
}

fn run_service() -> windows_service::Result<()> {
    debug!("Starting windows service for {SERVICE_NAME}");

    // Create a channel to be able to poll a stop event from the service worker loop.
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    // Define system service event handler that will be receiving service events.
    let event_handler = {
        let shutdown_tx = shutdown_tx.clone();
        move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                // Notifies a service to report its current status information to the service
                // control manager. Always return NoError even if not implemented.
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,

                // Handle stop
                ServiceControl::Stop => {
                    shutdown_tx.send(0).unwrap();
                    ServiceControlHandlerResult::NoError
                }

                _ => ServiceControlHandlerResult::NotImplemented,
            }
        }
    };

    // Register system service event handler.
    // The returned status handle should be used to report service status changes to the system.
    debug!("Registering service control handler for {SERVICE_NAME}");
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Tell the system that service is running
    debug!("Setting service status as running for {SERVICE_NAME}");
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Kick off thread to run our cli
    debug!("Spawning CLI thread for {SERVICE_NAME}");
    thread::spawn({
        move || {
            debug!("Loading CLI using args from disk for {SERVICE_NAME}");
            let config = match Config::load() {
                Ok(config) => config,
                Err(x) => {
                    error!("{x}");
                    shutdown_tx.send(x.to_i32()).unwrap();
                    return;
                }
            };

            debug!("Parsing CLI args from disk for {SERVICE_NAME}");
            let cli = match Cli::initialize_from(config.args) {
                Ok(cli) => cli,
                Err(x) => {
                    error!("{x}");
                    shutdown_tx.send(x.to_i32()).unwrap();
                    return;
                }
            };

            let logger = cli.init_logger();

            debug!("Running CLI for {SERVICE_NAME}");
            if let Err(x) = cli.run() {
                if !x.is_silent() {
                    error!("{}", x);
                }
                logger.flush();
                logger.shutdown();

                shutdown_tx.send(x.to_i32()).unwrap();
            } else {
                logger.flush();
                logger.shutdown();
                shutdown_tx.send(0).unwrap();
            }
        }
    });
    let code = loop {
        match shutdown_rx.recv_timeout(Duration::from_millis(100)) {
            // Break the loop either upon stop or channel disconnect
            Ok(code) => break code,
            Err(mpsc::RecvTimeoutError::Disconnected) => break 0,

            // Continue work if no events were received within the timeout
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        };
    };

    // Tell the system that service has stopped.
    debug!("Setting service status as stopped w/ code {code} for {SERVICE_NAME}");
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: if code == 0 {
            ServiceExitCode::NO_ERROR
        } else {
            ServiceExitCode::ServiceSpecific(code as u32)
        },
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
