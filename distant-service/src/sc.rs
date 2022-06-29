use std::{ffi::OsString, io, time::Duration};
use windows_service::{
    service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

pub struct ScServiceManager;

impl ServiceManager for ScServiceManager {
    fn start(&self) -> io::Result<()> {
        todo!();
    }

    fn stop(&self) -> io::Result<()> {
        todo!();
    }

    fn restart(&self) -> io::Result<()> {
        todo!();
    }

    fn install(&self) -> io::Result<()> {
        let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
        let service_binary_path = ::std::env::current_exe()
            .unwrap()
            .with_file_name("ping_service.exe");
        let service_info = ServiceInfo {
            name: OsString::from("ping_service"),
            display_name: OsString::from("Ping service"),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::OnDemand,
            error_control: ServiceErrorControl::Normal,
            executable_path: service_binary_path,
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None, // run as System
            account_password: None,
        };
        let service =
            service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
        service.set_description("Windows service example from windows-service-rs")?;
        Ok(())
    }

    fn uninstall(&self) -> io::Result<()> {
        let manager_access = ServiceManagerAccess::CONNECT;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

        let service_access =
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
        let service = service_manager.open_service("ping_service", service_access)?;

        let service_status = service.query_status()?;
        if service_status.current_state != ServiceState::Stopped {
            service.stop()?;
            // Wait for service to stop
            thread::sleep(Duration::from_secs(1));
        }

        service.delete()?;
        Ok(())
    }
}
