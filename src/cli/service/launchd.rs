use super::{Service, ServiceInstallCtx, ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx};
use crate::constants::HOME_DIR_PATH;
use once_cell::sync::Lazy;
use std::{io, path::PathBuf, process::Command};

static LAUNCHCTL: &str = "launchctl";

// For root (no login needed)
static GLOBAL_DAEMON_DIR_PATH: Lazy<PathBuf> =
    Lazy::new(|| PathBuf::from("/Library/LaunchDaemons"));

// For currently logged in user
static USER_AGENT_DIR_PATH: Lazy<PathBuf> =
    Lazy::new(|| HOME_DIR_PATH.join("Library").join("LaunchAgents"));

pub struct LaunchdService;

impl Service for LaunchdService {
    fn available(&self) -> io::Result<bool> {
        which::which(LAUNCHCTL)
            .map(|_| true)
            .map_err(|x| io::Error::new(io::ErrorKind::NotFound, x))
    }

    fn install(&self, ctx: ServiceInstallCtx) -> io::Result<()> {
        let dir_path = if ctx.user {
            USER_AGENT_DIR_PATH.as_path()
        } else {
            GLOBAL_DAEMON_DIR_PATH.as_path()
        };

        std::fs::create_dir_all(dir_path)?;

        let plist_path = dir_path.join(format!("{}.plist", ctx.label));
        let plist = make_plist(&ctx.label, ctx.cmd_iter());
        std::fs::write(plist_path.as_path(), plist)?;

        launchctl("load", plist_path.to_string_lossy().as_ref())
    }

    fn uninstall(&self, ctx: ServiceUninstallCtx) -> io::Result<()> {
        let dir_path = if ctx.user {
            USER_AGENT_DIR_PATH.as_path()
        } else {
            GLOBAL_DAEMON_DIR_PATH.as_path()
        };
        let plist_path = dir_path.join(format!("{}.plist", ctx.label));

        launchctl("unload", plist_path.to_string_lossy().as_ref())?;
        std::fs::remove_file(plist_path)
    }

    fn start(&self, ctx: ServiceStartCtx) -> io::Result<()> {
        launchctl("start", &ctx.label)
    }

    fn stop(&self, ctx: ServiceStopCtx) -> io::Result<()> {
        launchctl("stop", &ctx.label)
    }
}

fn launchctl(cmd: &str, label: &str) -> io::Result<()> {
    let output = Command::new(LAUNCHCTL).arg(cmd).arg(label).output()?;

    if output.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8(output.stderr)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("Failed to {cmd} for {label}"));

        Err(io::Error::new(io::ErrorKind::Other, msg))
    }
}

fn make_plist<'a>(label: &str, args: impl Iterator<Item = &'a str>) -> String {
    let args = args
        .map(|arg| format!("<string>{arg}</string>"))
        .collect::<Vec<String>>()
        .join("");
    format!(r#"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
    <dict>
        <key>Label</key>
        <string>{label}</string>
        <key>ProgramArguments</key>
        <array>
            {args}
        </array>
        <key>KeepAlive</key>
        <true/>
    </dict>
</plist>
"#).trim().to_string()
}
