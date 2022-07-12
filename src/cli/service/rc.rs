use super::{Service, ServiceInstallCtx, ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx};
use once_cell::sync::Lazy;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::Command,
};

static SERVICE_DIR_PATH: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("/etc/rc.d"));

/// Implementation of [`Service`] for FreeBSD's [rc.d](https://en.wikipedia.org/wiki/Init#Research_Unix-style/BSD-style)
pub struct RcService;

impl Service for RcService {
    fn available(&self) -> io::Result<bool> {
        match std::fs::metadata(SERVICE_DIR_PATH.as_path()) {
            Ok(_) => Ok(true),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(x) => Err(x),
        }
    }

    fn install(&self, ctx: ServiceInstallCtx) -> io::Result<()> {
        // NOTE: rc.d does not support user-level services
        if ctx.user {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "User-level services not supported for rc.d",
            ));
        }

        let service = ctx.label.to_script_name();
        let script = make_script(&service, &service, ctx.program.as_str(), ctx.args);

        // Create our script and ensure it is executable; fail if a script
        // exists at the location because we don't want to break something
        // and because OpenOptionsExt's mode(...) won't overwrite the
        // permissions of an existing file. We'd have to separately use
        // PermissionsExt to update those permissions if we wanted to
        // change an existing file's permissions
        //
        // NOTE: On Alpine Linux, /etc/init.d/{script} has permissions
        //       of rwxr-xr-x (755)
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o755)
            .open(rc_d_script_path(&service))?;
        file.write_all(script.as_bytes())?;

        rc_d_script("enable", &service)
    }

    fn uninstall(&self, ctx: ServiceUninstallCtx) -> io::Result<()> {
        let service = ctx.label.to_script_name();

        // Remove the service from rc.conf
        rc_d_script("delete", &service)?;

        // Delete the actual service file
        std::fs::remove_file(rc_d_script_path(&service))
    }

    fn start(&self, ctx: ServiceStartCtx) -> io::Result<()> {
        let service = ctx.label.to_script_name();
        rc_d_script("start", &service)
    }

    fn stop(&self, ctx: ServiceStopCtx) -> io::Result<()> {
        let service = ctx.label.to_script_name();
        rc_d_script("stop", &service)
    }
}

#[inline]
fn rc_d_script_path(name: &str) -> PathBuf {
    SERVICE_DIR_PATH.join(name)
}

fn rc_d_script(cmd: &str, service: &str) -> io::Result<()> {
    let output = Command::new(rc_d_script_path(service)).arg(cmd).output()?;

    if output.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8(output.stderr)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("Failed to {cmd} {service}"));

        Err(io::Error::new(io::ErrorKind::Other, msg))
    }
}

fn make_script(description: &str, provide: &str, program: &str, args: Vec<String>) -> String {
    let name = provide.replace('-', "_");
    let args = args.join(" ");
    format!(
        r#"
#!/bin/sh
#
# PROVIDE: {provide}
# REQUIRE: LOGIN FILESYSTEMS
# KEYWORD: shutdown

. /etc/rc.subr

name="{name}"
desc="{description}"
rcvar="{name}_enable"

load_rc_config ${{name}}

: ${{{name}_options="{args}"}}

pidfile="/var/run/{name}.pid"
procname="{program}"
command="/usr/sbin/daemon"
command_args="-c -S -T ${{name}} -p ${{pidfile}} ${{procname}} ${{{name}_options}}"

run_rc_command "$1"
    "#
    )
    .trim()
    .to_string()
}
