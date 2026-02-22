#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use derive_more::From;
use log::*;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use super::Cli;

const SERVICE_NAME: &str = "distant_manager";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    pub args: Vec<std::ffi::OsString>,
}

impl Config {
    pub fn save(&self) -> anyhow::Result<()> {
        let mut bytes = Vec::new();
        serde_json::to_writer(&mut bytes, self).context("Could not convert config into json")?;
        std::fs::write(Self::config_file(), bytes).context("Could not write config to file")
    }

    pub fn load() -> anyhow::Result<Self> {
        let bytes = std::fs::read(Self::config_file()).context("Could not read config file")?;
        serde_json::from_slice(&bytes).context("Could not convert json into config")
    }

    pub fn delete() -> anyhow::Result<()> {
        std::fs::remove_file(Self::config_file()).context("Could not delete config file")
    }

    /// Stored next to the service exe
    fn config_file() -> std::path::PathBuf {
        let mut path = std::env::current_exe().unwrap();
        path.set_extension("exe.config");
        path
    }
}

#[derive(From)]
pub enum ServiceError {
    /// Any other error type
    Anyhow(anyhow::Error),

    /// Represents a service-specific error that we use to known that we are not running as a
    /// service
    Service(windows_service::Error),
}

pub fn run() -> Result<(), ServiceError> {
    // Save our CLI arguments to pass on to the service
    let config = Config {
        args: std::env::args_os().collect(),
    };
    config.save()?;

    // Attempt to run as a service, deleting our config when completed
    // regardless of success
    let result = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    let config_result = Config::delete();

    // Swallow the config error if we have a service error, otherwise display
    // the config error
    match (result, config_result) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(x), _) => Err(ServiceError::Service(x)),
        (_, Err(x)) => Err(ServiceError::Anyhow(x)),
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
        move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                // Notifies a service to report its current status information to the service
                // control manager. Always return NoError even if not implemented.
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,

                // Handle stop
                ServiceControl::Stop => {
                    shutdown_tx.send(true).unwrap();
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
    let handle = thread::spawn({
        move || {
            debug!("Loading CLI using args from disk for {SERVICE_NAME}");
            let config = Config::load().expect("Failed to load config");

            debug!("Parsing CLI args from disk for {SERVICE_NAME}");
            let cli = Cli::initialize_from(config.args).expect("Failed to initialize CLI");

            debug!("Running CLI for {SERVICE_NAME}");
            cli.run().expect("CLI failed during execution")
        }
    });

    // Continually check for a shutdown trigger, catching completion of the thread
    // running our CLI as well and reporting errors if they occurred
    let success = loop {
        if handle.is_finished() {
            match handle.join() {
                Ok(_) => break true,
                Err(x) => {
                    error!("{x:?}");
                    break false;
                }
            }
        }

        match shutdown_rx.try_recv() {
            // Break the loop either upon stop or channel disconnect as a success
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => break true,

            // Continue work if no events were received within the timeout
            Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(100)),
        }
    };

    // Tell the system that service has stopped.
    debug!("Setting service status as stopped for {SERVICE_NAME}");
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: if success {
            ServiceExitCode::NO_ERROR
        } else {
            ServiceExitCode::ServiceSpecific(1u32)
        },
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
