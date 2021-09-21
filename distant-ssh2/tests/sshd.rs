use assert_fs::{prelude::*, TempDir};
use rstest::*;
use std::{
    collections::HashMap,
    fmt, io,
    path::Path,
    process::{Child, Command},
    sync::atomic::{AtomicU16, Ordering},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// NOTE: OpenSSH's sshd requires absolute path
const BIN_PATH_STR: &str = "/usr/sbin/sshd";

/// Port range to use when finding a port to bind to (using IANA guidance)
const PORT_RANGE: (u16, u16) = (49152, 65535);

pub struct SshKeygen;

impl SshKeygen {
    // ssh-keygen -t rsa -f $ROOT/id_rsa -N "" -q
    pub fn generate_rsa(path: impl AsRef<Path>, passphrase: impl AsRef<str>) -> io::Result<bool> {
        let res = Command::new("ssh-keygen")
            .args(&["-t", "rsa"])
            .arg("-f")
            .arg(path.as_ref())
            .arg("-N")
            .arg(passphrase.as_ref())
            .arg("-q")
            .status()
            .map(|status| status.success())?;

        #[cfg(unix)]
        if res {
            // chmod 600 id_rsa* -> ida_rsa + ida_rsa.pub
            std::fs::metadata(path.as_ref().with_extension("pub"))?
                .permissions()
                .set_mode(0o600);
            std::fs::metadata(path)?.permissions().set_mode(0o600);
        }

        Ok(res)
    }
}

pub struct SshAdd;

impl SshAdd {
    pub fn exec(path: impl AsRef<Path>) -> io::Result<bool> {
        Command::new("ssh-add")
            .arg(path.as_ref())
            .status()
            .map(|status| status.success())
    }
}

/* eval $(ssh-agent -s)

ssh-keygen -t rsa -f $SSHDIR/id_rsa -N "" -q
chmod 0600 $SSHDIR/id_rsa*
ssh-add $SSHDIR/id_rsa
cp $SSHDIR/id_rsa.pub $SSHDIR/authorized_keys

ssh-keygen -f $SSHDIR/ssh_host_rsa_key -N '' -t rsa */

#[derive(Debug)]
pub struct SshdConfig(HashMap<String, Vec<String>>);

impl Default for SshdConfig {
    fn default() -> Self {
        let mut config = Self::new();

        config.set_subsystem(true, true);
        config.set_use_pam(true);
        config.set_x11_forwarding(true);
        config.set_use_privilege_separation(false);
        config.set_print_motd(true);
        config.set_permit_tunnel(true);
        config.set_kbd_interactive_authentication(true);
        config.set_allow_tcp_forwarding(true);
        config.set_max_startups(500, None);
        config.set_strict_modes(false);

        config
    }
}

impl SshdConfig {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn set_authorized_keys_file(&mut self, path: impl AsRef<Path>) {
        self.0.insert(
            "AuthorizedKeysFile".to_string(),
            vec![path.as_ref().to_string_lossy().to_string()],
        );
    }

    pub fn set_host_key(&mut self, path: impl AsRef<Path>) {
        self.0.insert(
            "HostKey".to_string(),
            vec![path.as_ref().to_string_lossy().to_string()],
        );
    }

    pub fn set_pid_file(&mut self, path: impl AsRef<Path>) {
        self.0.insert(
            "PidFile".to_string(),
            vec![path.as_ref().to_string_lossy().to_string()],
        );
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

    pub fn set_strict_modes(&mut self, yes: bool) {
        self.0
            .insert("StrictModes".to_string(), Self::yes_value(yes));
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
    child: Child,

    /// Port that sshd is listening on
    pub port: u16,

    /// Temporary directory used to hold resources for sshd such as its config, keys, and log
    pub tmp: TempDir,
}

impl Sshd {
    pub fn spawn(mut config: SshdConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let tmp = TempDir::new()?;

        // ssh-keygen -t rsa -f $ROOT/id_rsa -N "" -q
        let id_rsa_file = tmp.child("id_rsa");
        assert!(
            SshKeygen::generate_rsa(id_rsa_file.path(), "")?,
            "Failed to ssh-keygen id_rsa"
        );
        assert!(
            SshAdd::exec(id_rsa_file.path())?,
            "Failed to ssh-add id_rsa"
        );

        // cp $ROOT/id_rsa.pub $ROOT/authorized_keys
        let authorized_keys_file = tmp.child("authorized_keys");
        std::fs::copy(
            id_rsa_file.path().with_extension("pub"),
            authorized_keys_file.path(),
        )?;

        // ssh-keygen -t rsa -f $ROOT/ssh_host_rsa_key -N "" -q
        let ssh_host_rsa_key_file = tmp.child("ssh_host_rsa_key");
        assert!(
            SshKeygen::generate_rsa(ssh_host_rsa_key_file.path(), "")?,
            "Failed to ssh-keygen ssh_host_rsa_key"
        );

        config.set_authorized_keys_file(id_rsa_file.path().with_extension("pub"));
        config.set_host_key(ssh_host_rsa_key_file.path());

        let sshd_pid_file = tmp.child("sshd.pid");
        config.set_pid_file(sshd_pid_file.path());

        // Generate $ROOT/sshd_config based on config
        let sshd_config_file = tmp.child("sshd_config");
        sshd_config_file.write_str(&config.to_string())?;

        let sshd_log_file = tmp.child("sshd.log");

        let (child, port) = Self::try_spawn_next(sshd_config_file.path(), sshd_log_file.path())
            .expect("No open port available for sshd");

        Ok(Self { child, port, tmp })
    }

    fn try_spawn_next(
        config_path: impl AsRef<Path>,
        log_path: impl AsRef<Path>,
    ) -> io::Result<(Child, u16)> {
        static PORT: AtomicU16 = AtomicU16::new(PORT_RANGE.0);

        loop {
            let port = PORT.fetch_add(1, Ordering::Relaxed);

            match Self::try_spawn(port, config_path.as_ref(), log_path.as_ref()) {
                // If successful, return our spawned server child process
                Ok(Ok(child)) => break Ok((child, port)),

                // If the server died when spawned, we don't want to keep trying it
                Ok(Err((code, msg))) => {
                    break Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("{} failed [{}]: {}", BIN_PATH_STR, code.unwrap_or(-1), msg),
                    ))
                }

                // If we've reached the final port in our range to try, we want to exit
                Err(x) if port == PORT_RANGE.1 => break Err(x),

                // Otherwise, try next port
                Err(_) => continue,
            }
        }
    }

    fn try_spawn(
        port: u16,
        config_path: impl AsRef<Path>,
        log_path: impl AsRef<Path>,
    ) -> io::Result<Result<Child, (Option<i32>, String)>> {
        let mut child = Command::new(BIN_PATH_STR)
            .arg("-p")
            .arg(port.to_string())
            .arg("-f")
            .arg(config_path.as_ref())
            .arg("-E")
            .arg(log_path.as_ref())
            .spawn()?;

        // Pause for couple of seconds to make sure that the server didn't die due to an error
        thread::sleep(Duration::from_secs(2));

        if let Some(exit_status) = child.try_wait()? {
            let output = child.wait_with_output()?;
            Ok(Err((
                exit_status.code(),
                String::from_utf8(output.stderr).unwrap(),
            )))
        } else {
            Ok(Ok(child))
        }
    }
}

impl Drop for Sshd {
    /// Kills server upon drop
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[fixture]
pub fn sshd() -> &'static Sshd {
    lazy_static::lazy_static! {
        static ref SSHD: Sshd = Sshd::spawn(Default::default()).unwrap();
    }

    &SSHD
}
