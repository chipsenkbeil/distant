use async_compat::CompatExt;
use std::{
    fmt, io,
    path::{Component, Path, Prefix},
    time::Duration,
};
use wezterm_ssh::{ExecResult, Session};

const READER_PAUSE_MILLIS: u64 = 100;

#[derive(Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl fmt::Debug for ExecOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let is_alternate = f.alternate();

        let mut s = f.debug_struct("ExecOutput");
        s.field("success", &self.success);

        if is_alternate {
            s.field("stdout", &String::from_utf8_lossy(&self.stdout))
                .field("stderr", &String::from_utf8_lossy(&self.stderr));
        } else {
            s.field("stdout", &self.stdout)
                .field("stderr", &self.stderr);
        }

        s.finish()
    }
}

pub async fn execute_output(session: &Session, cmd: &str) -> io::Result<ExecOutput> {
    let ExecResult {
        mut child,
        mut stdout,
        mut stderr,
        ..
    } = session
        .exec(cmd, None)
        .compat()
        .await
        .map_err(to_other_error)?;

    macro_rules! spawn_reader {
        ($reader:ident) => {{
            $reader.set_non_blocking(true).map_err(to_other_error)?;
            tokio::spawn(async move {
                use std::io::Read;
                let mut bytes = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    match $reader.read(&mut buf) {
                        Ok(n) if n > 0 => bytes.extend(&buf[..n]),
                        Ok(_) => break Ok(bytes),
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(READER_PAUSE_MILLIS)).await;
                        }
                        Err(x) => break Err(x),
                    }
                }
            })
        }};
    }

    // Spawn async readers for stdout and stderr from process
    let stdout_handle = spawn_reader!(stdout);
    let stderr_handle = spawn_reader!(stderr);

    // Wait for our handles to conclude
    let stdout = stdout_handle.await.map_err(to_other_error)??;
    let stderr = stderr_handle.await.map_err(to_other_error)??;

    // Wait for process to conclude
    let status = child.async_wait().compat().await.map_err(to_other_error)?;

    Ok(ExecOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

/// Convert a path into a string representing a unix path
///
/// E.g. C:\Users\example\Documents\file.txt -> /C/Users/example/Documents/file.txt
pub fn convert_path_to_unix_string(path: &Path) -> io::Result<String> {
    let mut s = String::new();
    for component in path.components() {
        s.push('/');

        match component {
            Component::Prefix(x) => match x.kind() {
                Prefix::Verbatim(x) => s.push_str(&x.to_string_lossy()),
                Prefix::VerbatimUNC(_, _) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Verbatim UNC not supported",
                    ));
                }
                Prefix::VerbatimDisk(x) => s.push(x as char),
                Prefix::DeviceNS(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Device NS not supported",
                    ));
                }
                Prefix::UNC(_, _) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "UNC not supported",
                    ));
                }
                Prefix::Disk(x) => s.push(x as char),
            },
            Component::RootDir => continue,
            Component::CurDir => s.push('.'),
            Component::ParentDir => s.push_str(".."),
            Component::Normal(x) => s.push_str(&x.to_string_lossy()),
        }
    }
    Ok(s)
}

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}
