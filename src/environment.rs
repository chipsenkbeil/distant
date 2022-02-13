use distant_core::SessionInfo;
use std::{ffi::OsStr, path::Path};

/// Prints out shell-specific environment information
pub fn print_environment(info: &SessionInfo) {
    inner_print_environment(&info.host, info.port, &info.key_to_unprotected_string())
}

/// Prints out shell-specific environment information
#[cfg(unix)]
fn inner_print_environment(host: &str, port: u16, key: &str) {
    match parent_exe_name() {
        // If shell is csh or tcsh, we want to print differently
        Some(s) if s.eq_ignore_ascii_case("csh") || s.eq_ignore_ascii_case("tcsh") => {
            formatter::print_csh_string(host, port, key)
        }

        // If shell is fish, we want to print differently
        Some(s) if s.eq_ignore_ascii_case("fish") => formatter::print_fish_string(host, port, key),

        // Otherwise, we assume that the shell is compatible with sh (e.g. bash, dash, zsh)
        _ => formatter::print_sh_string(host, port, key),
    }
}

/// Prints out shell-specific environment information
#[cfg(windows)]
fn inner_print_environment(host: &str, port: u16, key: &str) {
    match parent_exe_name() {
        // If shell is powershell, we want to print differently
        Some(p) if s.name().eq_ignore_ascii_case("powershell") => {
            formatter::print_powershell_string(host, port, key)
        }

        // Otherwise, we assume that the shell was cmd.exe
        _ => formatter::print_cmd_exe_string(host, port, key),
    }
}

/// Retrieve the name of the parent process that spawned us
fn parent_exe_name() -> Option<String> {
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

    maybe_parent_pid
        .and_then(|pid| system.process(pid))
        .map(Process::exe)
        .and_then(Path::file_name)
        .map(OsStr::to_string_lossy)
        .map(|s| s.to_string())
}

mod formatter {
    use indoc::printdoc;

    /// Prints out a {csh,tcsh}-specific example of setting environment variables
    #[cfg(unix)]
    pub fn print_csh_string(host: &str, port: u16, key: &str) {
        printdoc! {r#"
            setenv DISTANT_HOST "{host}"
            setenv DISTANT_PORT "{port}"
            setenv DISTANT_KEY "{key}"
            "#,
            host = host,
            port = port,
            key = key,
        }
    }

    /// Prints out a fish-specific example of setting environment variables
    #[cfg(unix)]
    pub fn print_fish_string(host: &str, port: u16, key: &str) {
        printdoc! {r#"
            # Please export the following variables to use with actions
            set -gx DISTANT_HOST {host}
            set -gx DISTANT_PORT {port}
            set -gx DISTANT_KEY {key}
            "#,
            host = host,
            port = port,
            key = key,
        }
    }

    /// Prints out an sh-compliant example of setting environment variables
    #[cfg(unix)]
    pub fn print_sh_string(host: &str, port: u16, key: &str) {
        printdoc! {r#"
            # Please export the following variables to use with actions
            export DISTANT_HOST="{host}"
            export DISTANT_PORT="{port}"
            export DISTANT_KEY="{key}"
            "#,
            host = host,
            port = port,
            key = key,
        }
    }

    /// Prints out a powershell example of setting environment variables
    #[cfg(windows)]
    pub fn print_powershell_string(host: &str, port: u16, key: &str) {
        printdoc! {r#"
            # Please export the following variables to use with actions
            $Env:DISTANT_HOST = "{host}"
            $Env:DISTANT_PORT = "{port}"
            $Env:DISTANT_KEY = "{key}"
            "#,
            host = host,
            port = port,
            key = key,
        }
    }

    /// Prints out a command prompt example of setting environment variables
    #[cfg(windows)]
    pub fn print_cmd_exe_string(host: &str, port: u16, key: &str) {
        printdoc! {r#"
            REM Please export the following variables to use with actions
            SET DISTANT_HOST="{host}"
            SET DISTANT_PORT="{port}"
            SET DISTANT_KEY="{key}"
            "#,
            host = host,
            port = port,
            key = key,
        }
    }
}
