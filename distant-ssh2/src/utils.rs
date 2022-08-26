use async_compat::CompatExt;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::Duration,
};
use wezterm_ssh::{ExecResult, Session, Sftp};

#[allow(dead_code)]
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

#[allow(dead_code)]
pub async fn execute_output(
    session: &Session,
    cmd: &str,
    timeout: Option<Duration>,
) -> io::Result<ExecOutput> {
    let ExecResult {
        mut child,
        mut stdout,
        mut stderr,
        stdin: _stdin,
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

    // Wait for process to conclude
    let status = child.async_wait().compat().await.map_err(to_other_error)?;

    // Wait for our handles to conclude (max of timeout if provided)
    let (stdout, stderr) = match timeout {
        Some(duration) => {
            let (res1, res2) = tokio::try_join!(
                tokio::time::timeout(duration, stdout_handle),
                tokio::time::timeout(duration, stderr_handle)
            )?;
            (res1??, res2??)
        }
        None => {
            let (res1, res2) = tokio::try_join!(stdout_handle, stderr_handle)?;
            (res1?, res2?)
        }
    };

    Ok(ExecOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}

/// Determines if using windows by checking the canonicalized path of '.'
pub async fn is_windows(session: &Session) -> io::Result<bool> {
    let output = execute_output(
        session,
        "cmd.exe /C echo %OS%",
        Some(Duration::from_secs(1)),
    )
    .await?;

    fn contains_subslice(slice: &[u8], subslice: &[u8]) -> bool {
        for i in 0..slice.len() {
            if i + subslice.len() > slice.len() {
                break;
            }

            if slice[i..].starts_with(subslice) {
                return true;
            }
        }

        false
    }

    Ok(contains_subslice(&output.stdout, b"Windows_NT")
        || contains_subslice(&output.stderr, b"Windows_NT"))
}

/// Performs canonicalization of the given path using SFTP
pub async fn canonicalize(sftp: &Sftp, path: impl AsRef<Path>) -> io::Result<PathBuf> {
    sftp.canonicalize(path.as_ref().to_path_buf())
        .compat()
        .await
        .map(|p| p.into_std_path_buf())
        .map_err(to_other_error)
}
