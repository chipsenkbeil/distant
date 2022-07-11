use super::{Service, ServiceInstallCtx, ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx};
use once_cell::sync::Lazy;
use std::{io, path::PathBuf, process::Command};

static RC_SERVICE: &str = "rc-service";
static RC_UPDATE: &str = "rc-update";
static SERVICE_DIR_PATH: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("/etc/init.d"));

pub struct OpenRcService;

impl Service for OpenRcService {
    fn available(&self) -> io::Result<bool> {
        which::which(RC_SERVICE)
            .map(|_| true)
            .map_err(|x| io::Error::new(io::ErrorKind::NotFound, x))
    }

    fn install(&self, ctx: ServiceInstallCtx) -> io::Result<()> {
        let dir_path = SERVICE_DIR_PATH.as_path();

        std::fs::create_dir_all(dir_path)?;

        let script_path = dir_path.join(&ctx.label);

        let script = make_script(
            &ctx.label,
            &ctx.label,
            ctx.program.as_str(),
            ctx.args,
            if ctx.user {
                Some(whoami::username())
            } else {
                None
            },
        );
        std::fs::write(script_path.as_path(), script)?;

        rc_update("add", &ctx.label)
    }

    fn uninstall(&self, ctx: ServiceUninstallCtx) -> io::Result<()> {
        rc_update("delete", &ctx.label)
    }

    fn start(&self, ctx: ServiceStartCtx) -> io::Result<()> {
        rc_service("start", &ctx.label)
    }

    fn stop(&self, ctx: ServiceStopCtx) -> io::Result<()> {
        rc_service("stop", &ctx.label)
    }
}

fn rc_service(cmd: &str, service: &str) -> io::Result<()> {
    let output = Command::new(RC_SERVICE).arg(service).arg(cmd).output()?;

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

fn rc_update(cmd: &str, service: &str) -> io::Result<()> {
    let output = Command::new(RC_UPDATE).arg(cmd).arg(service).output()?;

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

fn make_script(
    description: &str,
    provide: &str,
    program: &str,
    args: Vec<String>,
    user: Option<String>,
) -> String {
    format!(
        r#"
#!/sbin/openrc-run

description="{description}"
command="${{DISTANT_BINARY:-"{program}"}}"
command_args="{}"
command_background=true
{}

depend() {{
    provide {provide}
}}
    "#,
        args.join(" "),
        user.map(|user| format!(r#"command_user="{user}""#))
            .unwrap_or_default()
    )
    .trim()
    .to_string()
}
