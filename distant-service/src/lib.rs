use std::io;
mod launchd;
mod sc;

/// Interface to engaging with some service manager
pub trait ServiceManager {
    fn start(&self) -> io::Result<()>;
    fn stop(&self) -> io::Result<()>;
    fn restart(&self) -> io::Result<()>;

    fn install(&self) -> io::Result<()>;
    fn uninstall(&self) -> io::Result<()>;
}
