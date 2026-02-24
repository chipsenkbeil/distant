use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{fmt, io, thread};

use anyhow::Context;
use assert_fs::prelude::*;
use assert_fs::TempDir;
use derive_more::{Deref, DerefMut, Display};
use distant_core::Client;
use distant_ssh::{Ssh, SshAuthEvent, SshAuthHandler, SshOpts};
use log::*;
use once_cell::sync::Lazy;
use rstest::*;

use crate::utils::ci_path_to_string;

#[derive(Deref, DerefMut)]
pub struct Ctx<T> {
    #[deref]
    #[deref_mut]
    pub value: T,

    #[allow(dead_code)] // Used to keep sshd alive during tests
    pub sshd: Sshd,
}

// NOTE: Should find path
//
// Unix should be something like /usr/sbin/sshd
// Windows should be something like C:\Windows\System32\OpenSSH\sshd.exe
static BIN_PATH: Lazy<PathBuf> =
    Lazy::new(|| which::which(if cfg!(windows) { "sshd.exe" } else { "sshd" }).unwrap());

/// Port range to use when finding a port to bind to (using IANA guidance)
const PORT_RANGE: (u16, u16) = (49152, 65535);

pub static USERNAME: Lazy<String> = Lazy::new(whoami::username);

/// Time to wait after spawning sshd before continuing. Will check if still alive
const WAIT_AFTER_SPAWN: Duration = Duration::from_millis(300);

/// Maximum times to retry spawning sshd when it fails
const SPAWN_RETRY_CNT: usize = 3;

const MAX_DROP_WAIT_TIME: Duration = Duration::from_millis(500);

/// Sets restrictive Windows ACLs on a file so that Windows OpenSSH accepts it.
/// In admin contexts, removes inherited permissions and sets exact ACLs.
/// In non-admin contexts, additively grants permissions without stripping inherited ones.
#[cfg(windows)]
fn set_windows_file_permissions(path: &Path) {
    let current_user = whoami::username();
    let path_str = path.to_string_lossy();

    // Try to set SYSTEM ownership — only works as admin
    let is_admin = Command::new("icacls")
        .arg(&*path_str)
        .arg("/setowner")
        .arg("SYSTEM")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_admin {
        // Admin path: aggressive ACL setup (CI / elevated contexts)
        // Remove inherited + explicit broad perms, then grant exactly what's needed
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/inheritance:r")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/remove")
            .arg("*S-1-1-0")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/remove")
            .arg("*S-1-5-32-545")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant:r")
            .arg("SYSTEM:F")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant:r")
            .arg("Administrators:F")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant:r")
            .arg(format!("{}:RW", current_user))
            .output();
    } else {
        // Non-admin path: just add grants without stripping inherited perms
        warn!("Not admin — skipping aggressive ACL setup for {}", path_str);
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant")
            .arg("SYSTEM:F")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant")
            .arg("Administrators:F")
            .output();
        let _ = Command::new("icacls")
            .arg(&*path_str)
            .arg("/grant")
            .arg(format!("{}:F", current_user))
            .output();
    }
}

pub struct SshKeygen;

impl SshKeygen {
    // ssh-keygen -t ed25519 -f $ROOT/id_ed25519 -N "" -q
    pub fn generate_ed25519(
        path: impl AsRef<Path>,
        passphrase: impl AsRef<str>,
    ) -> anyhow::Result<bool> {
        let res = Command::new("ssh-keygen")
            .args(["-m", "PEM"])
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(path.as_ref())
            .arg("-N")
            .arg(passphrase.as_ref())
            .arg("-q")
            .status()
            .map(|status| status.success())
            .context("Failed to generate ed25519 key")?;

        if res {
            #[cfg(unix)]
            {
                // chmod 600 id_ed25519* -> ida_ed25519 + ida_ed25519.pub
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path.as_ref().with_extension("pub"))
                    .context("Failed to load metadata of ed25519 pub key")?
                    .permissions()
                    .set_mode(0o600);
                std::fs::metadata(path)
                    .context("Failed to load metadata of ed25519 key")?
                    .permissions()
                    .set_mode(0o600);
            }

            #[cfg(windows)]
            {
                let pub_key_path = path.as_ref().with_extension("pub");
                for key_path in [path.as_ref(), pub_key_path.as_path()] {
                    set_windows_file_permissions(key_path);
                }
            }
        }

        Ok(res)
    }
}

/// Log level for sshd config
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Hash)]
pub enum SshdLogLevel {
    #[display(fmt = "QUIET")]
    Quiet,
    #[display(fmt = "FATAL")]
    Fatal,
    #[display(fmt = "ERROR")]
    Error,
    #[display(fmt = "INFO")]
    Info,
    #[display(fmt = "VERBOSE")]
    Verbose,
    #[display(fmt = "DEBUG")]
    Debug,
    #[display(fmt = "DEBUG1")]
    Debug1,
    #[display(fmt = "DEBUG2")]
    Debug2,
    #[display(fmt = "DEBUG3")]
    Debug3,
}

#[derive(Debug)]
pub struct SshdConfig(HashMap<String, Vec<String>>);

impl Default for SshdConfig {
    fn default() -> Self {
        let mut config = Self::new();

        config.set_authentication_methods(vec!["publickey".to_string()]);
        config.set_pubkey_authentication(true);
        // UsePrivilegeSeparation and UsePAM are not supported by Windows OpenSSH
        if !cfg!(windows) {
            config.set_use_privilege_separation(false);
            config.set_use_pam(false);
        }
        config.set_subsystem(true, true);
        config.set_x11_forwarding(true);
        config.set_print_motd(true);
        config.set_permit_tunnel(true);
        config.set_kbd_interactive_authentication(true);
        config.set_allow_tcp_forwarding(true);
        config.set_max_startups(500, None);
        config.set_strict_modes(false);
        config.set_log_level(SshdLogLevel::Debug3);

        config
    }
}

impl SshdConfig {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn set_authentication_methods(&mut self, methods: Vec<String>) {
        self.0.insert("AuthenticationMethods".to_string(), methods);
    }

    pub fn set_authorized_keys_file(&mut self, path: impl AsRef<Path>) {
        let path = ci_path_to_string(path.as_ref());

        self.0.insert("AuthorizedKeysFile".to_string(), vec![path]);
    }

    pub fn set_host_key(&mut self, path: impl AsRef<Path>) {
        let path = ci_path_to_string(path.as_ref());

        self.0.insert("HostKey".to_string(), vec![path]);
    }

    pub fn set_pid_file(&mut self, path: impl AsRef<Path>) {
        let path = ci_path_to_string(path.as_ref());

        self.0.insert("PidFile".to_string(), vec![path]);
    }

    pub fn set_subsystem(&mut self, sftp: bool, internal_sftp: bool) {
        let mut values = Vec::new();
        if sftp {
            values.push("sftp".to_string());
        }
        if internal_sftp {
            values.push("internal-sftp".to_string());
        }

        self.0.insert("Subsystem".to_string(), values);
    }

    pub fn set_use_pam(&mut self, yes: bool) {
        self.0.insert("UsePAM".to_string(), Self::yes_value(yes));
    }

    pub fn set_x11_forwarding(&mut self, yes: bool) {
        self.0
            .insert("X11Forwarding".to_string(), Self::yes_value(yes));
    }

    pub fn set_use_privilege_separation(&mut self, yes: bool) {
        self.0
            .insert("UsePrivilegeSeparation".to_string(), Self::yes_value(yes));
    }

    pub fn set_print_motd(&mut self, yes: bool) {
        self.0.insert("PrintMotd".to_string(), Self::yes_value(yes));
    }

    pub fn set_permit_tunnel(&mut self, yes: bool) {
        self.0
            .insert("PermitTunnel".to_string(), Self::yes_value(yes));
    }

    pub fn set_kbd_interactive_authentication(&mut self, yes: bool) {
        self.0.insert(
            "KbdInteractiveAuthentication".to_string(),
            Self::yes_value(yes),
        );
    }

    pub fn set_allow_tcp_forwarding(&mut self, yes: bool) {
        self.0
            .insert("AllowTcpForwarding".to_string(), Self::yes_value(yes));
    }

    pub fn set_max_startups(&mut self, start: u16, rate_full: Option<(u16, u16)>) {
        let value = format!(
            "{}{}",
            start,
            rate_full
                .map(|(r, f)| format!(":{}:{}", r, f))
                .unwrap_or_default(),
        );

        self.0.insert("MaxStartups".to_string(), vec![value]);
    }

    pub fn set_pubkey_authentication(&mut self, yes: bool) {
        self.0
            .insert("PubkeyAuthentication".to_string(), Self::yes_value(yes));
    }

    pub fn set_strict_modes(&mut self, yes: bool) {
        self.0
            .insert("StrictModes".to_string(), Self::yes_value(yes));
    }

    pub fn set_log_level(&mut self, log_level: SshdLogLevel) {
        self.0
            .insert("LogLevel".to_string(), vec![log_level.to_string()]);
    }

    fn yes_value(yes: bool) -> Vec<String> {
        vec![Self::yes_string(yes)]
    }

    fn yes_string(yes: bool) -> String {
        Self::yes_str(yes).to_string()
    }

    const fn yes_str(yes: bool) -> &'static str {
        if yes {
            "yes"
        } else {
            "no"
        }
    }
}

impl fmt::Display for SshdConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (keyword, values) in self.0.iter() {
            writeln!(
                f,
                "{} {}",
                keyword,
                values
                    .iter()
                    .map(|v| {
                        let v = v.trim();
                        if v.contains(|c: char| c.is_whitespace()) {
                            format!("\"{}\"", v)
                        } else {
                            v.to_string()
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(" ")
            )?;
        }
        Ok(())
    }
}

/// Context for some sshd instance
pub struct Sshd {
    child: Mutex<Option<Child>>,

    /// Port that sshd is listening on
    pub port: u16,

    /// Temporary directory used to hold resources for sshd such as its config, keys, and log
    pub tmp: TempDir,

    /// Path to config file to print out when failures happen
    pub config_file: PathBuf,

    /// Path to log file to print out when failures happen
    pub log_file: PathBuf,
}

impl Sshd {
    /// Cached check if dead, does not actually do the check itself
    pub fn is_dead(&self) -> bool {
        self.child.lock().unwrap().is_none()
    }

    pub fn spawn(mut config: SshdConfig) -> anyhow::Result<Self> {
        let tmp = TempDir::new().context("Failed to create temporary directory")?;

        // ssh-keygen -t ed25519 -f $ROOT/id_ed25519 -N "" -q
        let id_ed25519_file = tmp.child("id_ed25519");
        assert!(
            SshKeygen::generate_ed25519(id_ed25519_file.path(), "")
                .context("Failed to generate ed25519 key for self")?,
            "Failed to ssh-keygen id_ed25519"
        );

        // cp $ROOT/id_ed25519.pub $ROOT/authorized_keys
        let authorized_keys_file = tmp.child("authorized_keys");
        std::fs::copy(
            id_ed25519_file.path().with_extension("pub"),
            authorized_keys_file.path(),
        )
        .context("Failed to copy ed25519 pub key to authorized keys file")?;

        // On Windows, grant SYSTEM + Administrators read access to authorized_keys
        // without stripping inherited permissions. The aggressive ACL setup used for
        // host/identity keys causes "Access is denied" on authorized_keys because sshd
        // holds the file open. With StrictModes=no, permissive ACLs are fine here.
        #[cfg(windows)]
        {
            let ak_path_str = authorized_keys_file.path().to_string_lossy().to_string();
            let _ = Command::new("icacls")
                .arg(&ak_path_str)
                .arg("/grant")
                .arg("SYSTEM:F")
                .output();
            let _ = Command::new("icacls")
                .arg(&ak_path_str)
                .arg("/grant")
                .arg("Administrators:F")
                .output();
        }

        // ssh-keygen -t ed25519 -f $ROOT/ssh_host_ed25519_key -N "" -q
        let ssh_host_ed25519_key_file = tmp.child("ssh_host_ed25519_key");
        assert!(
            SshKeygen::generate_ed25519(ssh_host_ed25519_key_file.path(), "")
                .context("Failed to generate ed25519 key for host")?,
            "Failed to ssh-keygen ssh_host_ed25519_key"
        );

        config.set_authorized_keys_file(&authorized_keys_file);
        config.set_host_key(ssh_host_ed25519_key_file.path());

        let sshd_pid_file = tmp.child("sshd.pid");
        config.set_pid_file(sshd_pid_file.path());

        // Generate $ROOT/sshd_config based on config
        let sshd_config_file = tmp.child("sshd_config");
        sshd_config_file
            .write_str(&config.to_string())
            .context("Failed to write sshd config to file")?;

        let sshd_log_file = tmp.child("sshd.log");

        let (child, port) = Self::try_spawn_next(sshd_config_file.path(), sshd_log_file.path())
            .context("Failed to find open port for sshd")?;

        Ok(Self {
            child: Mutex::new(Some(child)),
            port,
            tmp,
            config_file: sshd_config_file.to_path_buf(),
            log_file: sshd_log_file.to_path_buf(),
        })
    }

    fn try_spawn_next(
        config_path: impl AsRef<Path>,
        log_path: impl AsRef<Path>,
    ) -> anyhow::Result<(Child, u16)> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        static PORT: AtomicU16 = AtomicU16::new(0);

        // Initialize with a random-ish starting port on first use to reduce
        // port contention when nextest runs many test processes concurrently
        PORT.compare_exchange(
            0,
            {
                let mut hasher = DefaultHasher::new();
                std::process::id().hash(&mut hasher);
                std::thread::current().id().hash(&mut hasher);
                let hash = hasher.finish();
                let range_size = (PORT_RANGE.1 - PORT_RANGE.0) as u64;
                PORT_RANGE.0 + (hash % range_size) as u16
            },
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .ok();

        let max_port_attempts = 100;

        for _ in 0..max_port_attempts {
            let port = PORT.fetch_add(1, Ordering::Relaxed);
            // Wrap around if we exceed the range
            let port = PORT_RANGE.0 + ((port - PORT_RANGE.0) % (PORT_RANGE.1 - PORT_RANGE.0));

            match Self::try_spawn(port, config_path.as_ref(), log_path.as_ref()) {
                // If successful, return our spawned server child process
                Ok(Ok(child)) => return Ok((child, port)),

                // Otherwise, try next port
                Ok(Err((code, msg))) => {
                    error!(
                        "sshd could not spawn on port {port}, exited with code {:?}: {msg}, so trying next port",
                        code
                    );
                    if let Ok(log_content) = std::fs::read_to_string(&log_path) {
                        if !log_content.trim().is_empty() {
                            error!("SSHD LOG CONTENT for port {port}:\n{}", log_content);
                        }
                    }
                    continue;
                }
                Err(e) => {
                    error!("sshd could not spawn on port {port} due to error: {e}, so trying next port");
                    if let Ok(log_content) = std::fs::read_to_string(&log_path) {
                        if !log_content.trim().is_empty() {
                            error!("SSHD LOG CONTENT for port {port}:\n{}", log_content);
                        }
                    }
                    continue;
                }
            }
        }

        anyhow::bail!("Failed to find open port for sshd after {max_port_attempts} attempts")
    }

    fn try_spawn(
        port: u16,
        config_path: impl AsRef<Path>,
        log_path: impl AsRef<Path>,
    ) -> anyhow::Result<Result<Child, (Option<i32>, String)>> {
        // Sshd doesn't reliably fail when binding to a taken port, so we do a TCP check first
        // to try to ensure it is available
        drop(
            std::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), port))
                .with_context(|| format!("Port {port} already taken"))?,
        );

        #[cfg(windows)]
        {
            warn!(
                "Attempting to spawn sshd on Windows - this may require administrator privileges"
            );

            // Log the exact command being executed
            error!(
                "Spawning sshd with command: {:?} {:?}",
                BIN_PATH.as_path(),
                [
                    "-D",
                    "-p",
                    &port.to_string(),
                    "-f",
                    config_path.as_ref().to_string_lossy().as_ref(),
                    "-E",
                    log_path.as_ref().to_string_lossy().as_ref()
                ]
            );

            // Check if sshd binary exists and is accessible
            if let Ok(metadata) = std::fs::metadata(&*BIN_PATH) {
                error!(
                    "sshd binary info: path={:?}, size={}, readonly={}",
                    BIN_PATH.as_path(),
                    metadata.len(),
                    metadata.permissions().readonly()
                );
            } else {
                error!(
                    "sshd binary not found or not accessible at {:?}",
                    BIN_PATH.as_path()
                );
            }
        }

        let mut child = Command::new(BIN_PATH.as_path())
            .arg("-D")
            .arg("-p")
            .arg(port.to_string())
            .arg("-f")
            .arg(config_path.as_ref())
            .arg("-E")
            .arg(log_path.as_ref())
            .spawn()
            .with_context(|| {
                #[cfg(windows)]
                {
                    format!(
                        "Failed to spawn {:?}. On Windows Server 2025, sshd requires:\n\
                         1. Host key files owned by SYSTEM account\n\
                         2. Proper ACL permissions (SYSTEM:F, Administrators:F)\n\
                         3. No conflicting SSH services on the same port\n\
                         4. Administrator privileges for the test process\n\
                         \nTroubleshooting:\n\
                         - Check if system SSH service is running on port 22\n\
                         - Verify host key file permissions with 'icacls'\n\
                         - Ensure OpenSSH is properly installed",
                        BIN_PATH.as_path()
                    )
                }
                #[cfg(not(windows))]
                {
                    format!("Failed to spawn {:?}", BIN_PATH.as_path())
                }
            })?;

        // Check immediately for instant failures (like permission/config errors)
        if let Some(exit_status) = child
            .try_wait()
            .context("Failed to check sshd immediately")?
        {
            let output = child
                .wait_with_output()
                .context("Failed to get sshd output")?;
            error!(
                "sshd failed immediately with exit code {:?}",
                exit_status.code()
            );
            error!("sshd stdout: {}", String::from_utf8_lossy(&output.stdout));
            error!("sshd stderr: {}", String::from_utf8_lossy(&output.stderr));

            // Windows Server 2025 specific diagnostics on failure
            #[cfg(windows)]
            {
                // Detect Windows version
                if let Ok(output) = Command::new("ver").output() {
                    if let Ok(version) = String::from_utf8(output.stdout) {
                        error!("Windows version: {}", version.trim());
                        if version.contains("2025") || version.contains("26100") {
                            error!("Windows Server 2025 detected - requires SYSTEM file ownership for sshd");
                        }
                    }
                }

                // Check system SSH service conflicts
                if let Ok(output) = Command::new("sc").args(["query", "sshd"]).output() {
                    if let Ok(status) = String::from_utf8(output.stdout) {
                        if status.contains("RUNNING") {
                            error!("System SSH service is RUNNING - may conflict with test sshd instances");
                        } else if status.contains("STOPPED") {
                            error!("System SSH service is STOPPED");
                        }
                    }
                } else {
                    error!("Could not check system SSH service status");
                }

                // Check Windows OpenSSH version
                if let Ok(output) = Command::new(BIN_PATH.as_path()).arg("-V").output() {
                    if let Ok(version) = String::from_utf8(output.stderr) {
                        // SSH version goes to stderr
                        error!("OpenSSH version: {}", version.trim());
                    }
                }
            }

            // Also print log file for immediate failures
            if let Ok(log_content) = std::fs::read_to_string(&log_path) {
                if !log_content.trim().is_empty() {
                    error!("sshd log file content:\n{}", log_content);
                }
            }

            return Ok(Err((
                exit_status.code(),
                "Immediate failure after spawn".to_string(),
            )));
        }

        // Pause for a little bit to make sure that the server didn't die due to an error
        thread::sleep(Duration::from_millis(100));

        let child = match check(child).context("Sshd encountered problems (after 100ms)")? {
            Ok(child) => child,
            Err(x) => return Ok(Err(x)),
        };

        // Pause for a little bit to make sure that the server didn't die due to an error
        thread::sleep(Duration::from_millis(100));

        let result = check(child).context("Sshd encountered problems (after 200ms)")?;
        Ok(result)
    }

    /// Checks if still alive
    fn check_is_alive(&self) -> bool {
        // Check if our sshd process is still running, or if it died and we can report about it
        let mut child_lock = self.child.lock().unwrap();
        if let Some(child) = child_lock.take() {
            match check(child) {
                Ok(Ok(child)) => {
                    child_lock.replace(child);
                    true
                }
                Ok(Err((code, msg))) => {
                    error!(
                        "sshd died w/ exit code {}: {msg}",
                        if let Some(code) = code {
                            code.to_string()
                        } else {
                            "[missing]".to_string()
                        }
                    );
                    false
                }
                Err(x) => {
                    error!("Failed to check status of sshd: {x}");
                    false
                }
            }
        } else {
            error!("sshd is dead!");
            false
        }
    }

    fn print_log_file(&self) {
        match std::fs::read_to_string(&self.log_file) {
            Ok(log) if !log.trim().is_empty() => {
                let mut out = String::new();
                out.push('\n');
                out.push_str("====================\n");
                out.push_str("= SSHD LOG FILE     \n");
                out.push_str("====================\n");
                out.push('\n');
                out.push_str(&log);
                out.push('\n');
                out.push('\n');

                // Add Windows-specific diagnostic information
                #[cfg(windows)]
                {
                    out.push_str("= WINDOWS DIAGNOSTICS\n");
                    out.push_str("====================\n");

                    // Check if this is Windows Server 2025 which has stricter requirements
                    if let Ok(output) = std::process::Command::new("ver").output() {
                        if let Ok(version) = String::from_utf8(output.stdout) {
                            out.push_str(&format!("Windows Version: {}\n", version.trim()));
                            if version.contains("2025") || version.contains("26100") {
                                out.push_str("Detected Windows Server 2025 - requires SYSTEM file ownership\n");
                            }
                        }
                    }

                    // Check if system SSH service is running
                    if let Ok(output) = std::process::Command::new("sc")
                        .args(["query", "sshd"])
                        .output()
                    {
                        if let Ok(status) = String::from_utf8(output.stdout) {
                            if status.contains("RUNNING") {
                                out.push_str("System SSH service is RUNNING - may conflict with test instances\n");
                            }
                        }
                    }

                    out.push('\n');
                }

                out.push_str("====================\n");
                out.push('\n');
                error!("{out}");
            }
            Ok(_) => {
                error!("SSHD LOG FILE is empty (path: {:?})", self.log_file);
            }
            Err(e) => {
                error!("Failed to read SSHD LOG FILE at {:?}: {}", self.log_file, e);
            }
        }
    }

    fn print_config_file(&self) {
        if let Ok(contents) = std::fs::read_to_string(&self.config_file) {
            let mut out = String::new();
            out.push('\n');
            out.push_str("====================\n");
            out.push_str("= SSHD CONFIG FILE     \n");
            out.push_str("====================\n");
            out.push('\n');
            out.push_str(&contents);
            out.push('\n');
            out.push('\n');
            out.push_str("====================\n");
            out.push('\n');
            error!("{out}");
        }
    }
}

impl Drop for Sshd {
    /// Kills server upon drop
    fn drop(&mut self) {
        debug!("Dropping sshd");
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();

            // Wait for a maximum period of time
            let start = Instant::now();
            while start.elapsed() < MAX_DROP_WAIT_TIME {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        debug!("Sshd finished");
                        return;
                    }
                    Err(x) => {
                        error!("Failed to wait for sshd to quit: {x}");
                        return;
                    }
                    _ => thread::sleep(MAX_DROP_WAIT_TIME / 10),
                }
            }

            error!("Timed out waiting for sshd to quit");
        }
    }
}

/// Mocked version of [`SshAuthHandler`]
pub struct MockSshAuthHandler;

impl SshAuthHandler for MockSshAuthHandler {
    async fn on_authenticate(&self, event: SshAuthEvent) -> io::Result<Vec<String>> {
        debug!("on_authenticate: {:?}", event);
        Ok(vec![String::new(); event.prompts.len()])
    }

    async fn on_verify_host(&self, host: &str) -> io::Result<bool> {
        debug!("on_host_verify: {}", host);
        Ok(true)
    }

    async fn on_banner(&self, text: &str) {
        debug!("on_banner: {:?}", text);
    }

    async fn on_error(&self, text: &str) {
        debug!("on_error: {:?}", text);
    }
}

#[fixture]
pub fn sshd() -> Sshd {
    let mut i = 0;
    loop {
        if i == SPAWN_RETRY_CNT {
            panic!("Exceeded retry count!");
        }

        match Sshd::spawn(Default::default()) {
            // Succeeded, so wait a bit, check that is still alive, and then continue
            Ok(sshd) => {
                std::thread::sleep(WAIT_AFTER_SPAWN);

                if !sshd.check_is_alive() {
                    // We want to print out the log file from sshd in case it sheds clues on problem
                    sshd.print_log_file();

                    // We want to print out the config file from sshd in case it sheds clues on problem
                    sshd.print_config_file();

                    // Skip this spawn and try again
                    continue;
                }

                return sshd;
            }

            // Last attempt failed, so panic with the error encountered
            Err(x) if i + 1 == SPAWN_RETRY_CNT => panic!("{x}"),

            // Not last attempt, so sleep and then try again
            Err(_) => std::thread::sleep(WAIT_AFTER_SPAWN),
        }

        i += 1;
    }
}

/// Fixture to establish a client to an SSH server
#[fixture]
pub async fn client(sshd: Sshd) -> Ctx<Client> {
    let ssh_client = load_ssh_client(&sshd).await;
    let mut client = ssh_client
        .into_distant_client()
        .await
        .context("Failed to convert into distant client")
        .unwrap();
    client.shutdown_on_drop(true);
    Ctx {
        sshd,
        value: client,
    }
}

/// Access to raw [`Ssh`] client
#[fixture]
pub async fn ssh(sshd: Sshd) -> Ctx<Ssh> {
    let ssh = load_ssh_client(&sshd).await;
    Ctx { sshd, value: ssh }
}

pub async fn load_ssh_client(sshd: &Sshd) -> Ssh {
    if sshd.is_dead() {
        panic!("sshd is dead!");
    }

    let port = sshd.port;
    let opts = SshOpts {
        port: Some(port),
        identity_files: vec![sshd.tmp.child("id_ed25519").path().to_path_buf()],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        // verbose: true,
        ..Default::default()
    };

    let addrs = vec![
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ];
    let mut errors = Vec::new();
    let msg = format!("Failed to connect to any of these hosts: {addrs:?}");

    let max_attempts = 3;

    for attempt in 1..=max_attempts {
        for addr in &addrs {
            let addr_string = addr.to_string();
            match Ssh::connect(&addr_string, opts.clone()).await {
                Ok(mut ssh_client) => match ssh_client.authenticate(MockSshAuthHandler).await {
                    Ok(_) => return ssh_client,
                    Err(x) => {
                        errors.push(anyhow::Error::new(x).context(format!(
                            "Failed to authenticate with sshd @ {addr_string} (attempt {attempt})"
                        )));
                    }
                },
                Err(x) => {
                    errors.push(anyhow::Error::new(x).context(format!(
                        "Failed to connect to sshd @ {addr_string} (attempt {attempt})"
                    )));
                }
            }
        }

        if attempt < max_attempts {
            warn!("SSH auth attempt {attempt} failed, retrying after 500ms...");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Verify sshd is still alive before retrying
            if sshd.is_dead() {
                break;
            }
        }
    }

    // Check if still alive, which will print out messages
    if sshd.check_is_alive() {
        warn!("sshd is still alive, so something else is going on");
    }

    // Kill sshd so we can read its log file (Windows locks it while sshd runs)
    {
        let mut child_lock = sshd.child.lock().unwrap();
        if let Some(mut child) = child_lock.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    // We want to print out the log file from sshd in case it sheds clues on problem
    sshd.print_log_file();

    // We want to print out the config file from sshd in case it sheds clues on problem
    sshd.print_config_file();

    // On Windows, dump ACLs for key files to diagnose permission issues
    #[cfg(windows)]
    {
        for name in ["authorized_keys", "id_ed25519", "id_ed25519.pub"] {
            let key_path = sshd.tmp.child(name).path().to_path_buf();
            match Command::new("icacls").arg(&key_path).output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    error!(
                        "icacls {} => stdout: {}, stderr: {}",
                        name,
                        stdout.trim(),
                        stderr.trim()
                    );
                }
                Err(e) => {
                    error!("Failed to run icacls on {}: {}", name, e);
                }
            }
        }
    }

    let error = match errors.into_iter().reduce(|x, y| x.context(y)) {
        Some(x) => x.context(msg),
        None => anyhow::anyhow!(msg),
    };

    panic!("{error:?}");
}

fn check(mut child: Child) -> anyhow::Result<Result<Child, (Option<i32>, String)>> {
    if let Some(exit_status) = child.try_wait().context("Failed to check status of sshd")? {
        let output = child.wait_with_output().context("Failed to wait on sshd")?;
        Ok(Err((
            exit_status.code(),
            format!(
                "{}\n{}",
                String::from_utf8(output.stdout).unwrap(),
                String::from_utf8(output.stderr).unwrap(),
            ),
        )))
    } else {
        Ok(Ok(child))
    }
}
