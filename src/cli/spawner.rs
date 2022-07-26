use anyhow::Context;
use log::*;
use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

/// Utility functions to spawn a process in the background
#[allow(dead_code)]
pub struct Spawner;

#[allow(dead_code)]
impl Spawner {
    /// Spawns a new instance of this running process without a `--daemon` flag,
    /// returning the id of the spawned process
    pub fn spawn_running_background(extra_args: Vec<OsString>) -> anyhow::Result<u32> {
        let cmd = Self::make_current_cmd(extra_args, "--daemon")?;

        #[cfg(windows)]
        let cmd = {
            let mut s = OsString::new();
            s.push("'");
            s.push(&cmd);
            s.push("'");
            s
        };

        Self::spawn_background(cmd)
    }

    #[inline]
    fn make_current_cmd(extra_args: Vec<OsString>, exclude: &str) -> anyhow::Result<OsString> {
        // Get absolute path to our binary
        let program = which::which(std::env::current_exe().unwrap_or_else(|_| {
            PathBuf::from(if cfg!(windows) {
                "distant.exe"
            } else {
                "distant"
            })
        }))
        .context("Failed to locate distant binary")?;

        // Remove --daemon argument to to ensure runs in foreground,
        // otherwise we would fork bomb ourselves
        //
        // Also, remove first argument (program) since we determined it above
        let mut cmd = OsString::new();
        cmd.push(program.as_os_str());

        let it = std::env::args_os()
            .skip(1)
            .filter(|arg| {
                !arg.to_str()
                    .map(|s| s.trim().eq_ignore_ascii_case(exclude))
                    .unwrap_or_default()
            })
            .chain(extra_args.into_iter());
        for arg in it {
            cmd.push(" ");
            cmd.push(&arg);
        }

        Ok(cmd)
    }
}

#[cfg(unix)]
#[allow(dead_code)]
impl Spawner {
    /// Spawns a process on Unix that runs in the background and won't be terminated when the
    /// parent process exits
    pub fn spawn_background(cmd: impl AsRef<OsStr>) -> anyhow::Result<u32> {
        let cmd = cmd
            .as_ref()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("cmd is not a UTF-8 str"))?;

        // Build out the command and args from our string
        let (cmd, args) = match cmd.split_once(' ') {
            Some((cmd_str, args_str)) => (
                cmd_str,
                shell_words::split(args_str).context("Failed to split process arguments")?,
            ),
            None => (cmd, Vec::new()),
        };

        debug!("Spawning background process: {}", cmd);
        let child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn background process")?;
        Ok(child.id())
    }
}

#[cfg(windows)]
impl Spawner {
    /// Spawns a process on Windows that runs in the background without a console and does not get
    /// terminated when the parent or other ancestors terminate (such as openssh session)
    pub fn spawn_background(cmd: impl AsRef<OsStr>) -> anyhow::Result<u32> {
        use std::{
            io::{BufRead, Cursor},
            os::windows::process::CommandExt,
        };

        // Get absolute path to powershell
        let powershell = which::which("powershell.exe").context("Failed to find powershell.exe")?;

        // Pass along our environment variables
        let env = {
            let mut s = OsString::new();
            s.push(r#"$startup.Properties['EnvironmentVariables'].value=@("#);
            let mut first = true;
            for (key, value) in std::env::vars_os() {
                if !first {
                    s.push(",");
                } else {
                    first = false;
                }

                s.push("'");
                s.push(key);
                s.push("=");
                s.push(value);
                s.push("'");
            }
            s.push(")");
            s
        };

        let args = vec![
            OsString::from(r#"$startup=[wmiclass]"Win32_ProcessStartup""#),
            OsString::from(";"),
            OsString::from(r#"$startup.Properties['ShowWindow'].value=$False"#),
            OsString::from(";"),
            env,
            OsString::from(";"),
            OsString::from("Invoke-WmiMethod"),
            OsString::from("-Class"),
            OsString::from("Win32_Process"),
            OsString::from("-Name"),
            OsString::from("Create"),
            OsString::from("-ArgumentList"),
            {
                let mut arg_list = OsString::new();
                arg_list.push(cmd.as_ref());
                arg_list.push(",$null,$startup");
                arg_list
            },
        ];

        // const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;

        debug!(
            "Spawning background process: {} {:?}",
            powershell.to_string_lossy(),
            args
        );
        let output = Command::new(powershell.into_os_string())
            .creation_flags(flags)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("Failed to spawn background process")?;

        if !output.status.success() {
            anyhow::bail!(
                "Program failed [{}]: {}",
                output.status.code().unwrap_or(1),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = Cursor::new(output.stdout);

        let mut process_id = None;
        let mut return_value = None;
        for line in stdout.lines().filter_map(|l| l.ok()) {
            let line = line.trim();
            if line.starts_with("ProcessId") {
                if let Some((_, id)) = line.split_once(':') {
                    process_id = id.trim().parse::<u32>().ok();
                }
            } else if line.starts_with("ReturnValue") {
                if let Some((_, value)) = line.split_once(':') {
                    return_value = value.trim().parse::<i32>().ok();
                }
            }
        }

        match (return_value, process_id) {
            (Some(0), Some(pid)) => Ok(pid),
            (Some(0), None) => anyhow::bail!("Program succeeded, but missing process pid"),
            (Some(code), _) => anyhow::bail!(
                "Program failed [{}]: {}",
                code,
                String::from_utf8_lossy(&output.stderr)
            ),
            (None, _) => anyhow::bail!("Missing return value"),
        }
    }
}
