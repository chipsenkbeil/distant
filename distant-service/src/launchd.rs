use std::io;

/// Service manager implementation for Mac OS's [`launchd`]
pub struct LaunchdServiceManager;

impl ServiceManager for LaunchdServiceManager {
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
        todo!();
    }

    fn uninstall(&self) -> io::Result<()> {
        todo!();
    }
}
