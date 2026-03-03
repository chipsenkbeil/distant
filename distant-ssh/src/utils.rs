use std::fmt;
use std::io;
use std::time::Duration;

use russh::client::Handle;

use crate::ClientHandler;
use crate::SshFamily;

const SSH_EXEC_TIMEOUT: Option<Duration> = Some(Duration::from_secs(30));

#[derive(Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl std::fmt::Debug for ExecOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

pub async fn powershell_output(
    handle: &Handle<ClientHandler>,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    let cmd = format!("powershell.exe -NonInteractive -Command \"& {{{cmd}}}\"");
    execute_output(handle, &cmd, timeout).await
}

pub async fn execute_output(
    handle: &Handle<ClientHandler>,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    use russh::ChannelMsg;

    let timeout_duration = timeout.into();

    // Open a channel
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(io::Error::other)?;

    // Execute command
    channel.exec(true, cmd).await.map_err(io::Error::other)?;

    let read_future = async {
        // Read output via channel messages
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status: Option<u32> = None;
        let mut got_eof = false;

        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        stderr.extend_from_slice(data);
                    }
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                    if got_eof {
                        break;
                    }
                }
                ChannelMsg::Eof => {
                    got_eof = true;
                    if exit_status.is_some() {
                        break;
                    }
                }
                _ => {}
            }
        }

        Ok(ExecOutput {
            success: exit_status.map(|s| s == 0).unwrap_or(false),
            stdout,
            stderr,
        })
    };

    if let Some(duration) = timeout_duration {
        tokio::time::timeout(duration, read_future)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SSH command timed out"))?
    } else {
        read_future.await
    }
}

/// Query remote system for name of current user
pub async fn query_username(
    handle: &Handle<ClientHandler>,
    is_windows: bool,
) -> io::Result<String> {
    if is_windows {
        // Will get DOMAIN\USERNAME as output -- needed because USERNAME isn't set on
        // Github's Windows CI (it sets USER instead)
        let output = powershell_output(
            handle,
            "[System.Security.Principal.WindowsIdentity]::GetCurrent().Name",
            SSH_EXEC_TIMEOUT,
        )
        .await?;

        let output = String::from_utf8_lossy(&output.stdout);
        let output = match output.split_once('\\') {
            Some((_, username)) => username,
            None => output.as_ref(),
        };

        Ok(output.trim().to_string())
    } else {
        let output = execute_output(handle, "/bin/sh -c whoami", SSH_EXEC_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Query remote system for the default shell of current user
pub async fn query_shell(handle: &Handle<ClientHandler>, is_windows: bool) -> io::Result<String> {
    let output = if is_windows {
        powershell_output(
            handle,
            "[Environment]::GetEnvironmentVariable('ComSpec')",
            SSH_EXEC_TIMEOUT,
        )
        .await?
    } else {
        execute_output(handle, "/bin/sh -c 'echo $SHELL'", SSH_EXEC_TIMEOUT).await?
    };

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Determines if using windows by checking the OS environment variable
pub async fn is_windows(handle: &Handle<ClientHandler>) -> io::Result<bool> {
    let output = powershell_output(
        handle,
        "[Environment]::GetEnvironmentVariable('OS')",
        SSH_EXEC_TIMEOUT,
    )
    .await?;

    fn contains_subslice(slice: &[u8], subslice: &[u8]) -> bool {
        if subslice.is_empty() {
            return true;
        }
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

/// An owned path in SFTP wire format, aware of the remote platform.
///
/// SFTP always uses Unix-style (`/`) separators. On Windows targets,
/// native paths like `C:\Users\foo` are stored as `/C:/Users/foo`
/// (matching the OpenSSH SFTP wire format).
/// Unix paths pass through unchanged.
///
/// # Examples
///
/// ```
/// use distant_ssh::{SshFamily, SftpPathBuf};
/// use distant_core::protocol::RemotePath;
///
/// // Unix: passthrough
/// let sftp = SftpPathBuf::from_remote(&RemotePath::new("/home/user"), SshFamily::Unix);
/// assert_eq!(sftp.as_str(), "/home/user");
///
/// // Windows: native → SFTP format (drive prefix preserved)
/// let sftp = SftpPathBuf::from_remote(&RemotePath::new("C:\\Users\\foo"), SshFamily::Windows);
/// assert_eq!(sftp.as_str(), "/C:/Users/foo");
///
/// // SFTP response → native RemotePath
/// let sftp = SftpPathBuf::from_sftp("/C:/Users/foo", SshFamily::Windows);
/// assert_eq!(sftp.to_remote_path(), RemotePath::new("C:\\Users\\foo"));
///
/// // Relative paths use native separators
/// let sftp = SftpPathBuf::from_sftp("sub1", SshFamily::Windows);
/// let joined = sftp.join("file2");
/// assert_eq!(joined.to_remote_path(), RemotePath::new("sub1\\file2"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SftpPathBuf {
    /// Path in SFTP wire format (Unix-style separators).
    inner: String,
    /// Remote platform family.
    family: SshFamily,
}

impl SftpPathBuf {
    /// Create from a [`RemotePath`] (native format) and remote platform family.
    ///
    /// Converts native path separators to SFTP wire format:
    /// - **Unix:** passthrough (already uses `/`).
    /// - **Windows:** converts `C:\Users\foo` to `/C:/Users/foo`.
    pub fn from_remote(path: &distant_core::protocol::RemotePath, family: SshFamily) -> Self {
        let s = path.as_str();

        let inner = match family {
            SshFamily::Unix => {
                // Unix paths pass through unchanged — already use `/`.
                s.to_string()
            }
            SshFamily::Windows => {
                use typed_path::{Utf8WindowsComponent, Utf8WindowsPath};

                // Explicitly parse as a Windows path so `\` is recognized as
                // a separator even when this code runs on Unix.
                let p = Utf8WindowsPath::new(s);

                if let Some(Utf8WindowsComponent::Prefix(prefix)) = p.components().next() {
                    // `with_unix_encoding()` produces `/Users/foo` (root + normals).
                    // Prepend the drive letter: `/C:/Users/foo`.
                    let unix = p.with_unix_encoding();
                    format!("/{}{}", prefix.as_str(), unix)
                } else {
                    // Relative path: just convert `\` → `/`.
                    p.with_unix_encoding().to_string()
                }
            }
        };

        Self { inner, family }
    }

    /// Wrap an SFTP-returned string as an [`SftpPathBuf`].
    ///
    /// The string is assumed to already be in SFTP wire format (e.g. `/C:/Users/foo`).
    pub fn from_sftp(s: impl Into<String>, family: SshFamily) -> Self {
        Self {
            inner: s.into(),
            family,
        }
    }

    /// Returns the path in SFTP wire format for passing to SFTP API methods.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Convert to a native-format [`RemotePath`].
    ///
    /// - **Unix:** returns the inner string as-is.
    /// - **Windows:** strips leading `/` before a drive letter (e.g. `/C:/...` → `C:/...`),
    ///   then replaces all `/` with `\`.
    pub fn to_remote_path(&self) -> distant_core::protocol::RemotePath {
        distant_core::protocol::RemotePath::new(self.to_native_string())
    }

    /// Convert the SFTP path to a native path string.
    fn to_native_string(&self) -> String {
        if self.family == SshFamily::Windows {
            // Strip leading `/` before a drive letter: `/C:/...` → `C:/...`
            let stripped = self
                .inner
                .strip_prefix('/')
                .filter(|s| {
                    s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.get(1..2) == Some(":")
                })
                .unwrap_or(&self.inner);
            stripped.replace('/', "\\")
        } else {
            self.inner.clone()
        }
    }

    /// Consume and return the inner SFTP-format string.
    pub fn into_string(self) -> String {
        self.inner
    }

    /// Join a child component onto this path.
    ///
    /// Always joins with `/` (SFTP format). The result remains in SFTP wire format.
    pub fn join(&self, child: &str) -> SftpPathBuf {
        let inner = if self.inner.is_empty() {
            child.to_string()
        } else if self.inner.ends_with('/') {
            format!("{}{}", self.inner, child)
        } else {
            format!("{}/{}", self.inner, child)
        };
        SftpPathBuf {
            inner,
            family: self.family,
        }
    }

    /// Extract the file name (last component) from the path.
    ///
    /// Splits on both `/` and `\` to handle mixed separators.
    /// Trailing separators are ignored (e.g. `/home/user/` yields `user`).
    pub fn file_name(&self) -> Option<&str> {
        self.inner.rsplit(['/', '\\']).find(|s| !s.is_empty())
    }

    /// Strip a prefix from this path, returning the relative remainder.
    ///
    /// Both paths are normalized to `/` before comparison. A trailing `/` is
    /// added to the prefix if needed so that stripping `/home` from `/home/user`
    /// yields `user` (not `/user`).
    pub fn strip_prefix(&self, prefix: &SftpPathBuf) -> Option<String> {
        let path_normalized = self.inner.replace('\\', "/");
        let prefix_normalized = prefix.inner.replace('\\', "/");
        let prefix_with_sep = if prefix_normalized.ends_with('/') {
            prefix_normalized
        } else {
            format!("{prefix_normalized}/")
        };
        path_normalized
            .strip_prefix(&prefix_with_sep)
            .map(|s| s.to_string())
    }
}

impl fmt::Display for SftpPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl From<SftpPathBuf> for String {
    fn from(p: SftpPathBuf) -> String {
        p.into_string()
    }
}

#[cfg(test)]
mod tests {
    //! Tests for utility functions: `ExecOutput` Debug/equality behavior, constants, and
    //! `contains_subslice`.
    //!
    //! The `contains_subslice` function is replicated from the private function
    //! defined inside `is_windows()`, since it is not directly accessible from test
    //! code. If the production function diverges, these tests will not detect it.

    use super::*;

    // --- ExecOutput Debug format tests ---

    #[test]
    fn exec_output_debug_alternate_format() {
        let output = ExecOutput {
            success: true,
            stdout: b"hello".to_vec(),
            stderr: b"world".to_vec(),
        };
        let debug_str = format!("{:#?}", output);
        assert!(
            debug_str.contains("hello"),
            "Missing stdout in alternate debug: {}",
            debug_str
        );
        assert!(
            debug_str.contains("world"),
            "Missing stderr in alternate debug: {}",
            debug_str
        );
    }

    #[test]
    fn exec_output_debug_normal_format() {
        let output = ExecOutput {
            success: false,
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
        };
        let debug_str = format!("{:?}", output);
        assert!(
            debug_str.contains("success: false"),
            "Missing success field: {}",
            debug_str
        );
    }

    #[test]
    fn exec_output_alternate_debug_with_invalid_utf8() {
        let output = ExecOutput {
            success: true,
            stdout: vec![0xff, 0xfe, 0x41],
            stderr: vec![0x42, 0xff],
        };
        let alt_debug = format!("{:#?}", output);
        // from_utf8_lossy should produce replacement characters
        assert!(
            alt_debug.contains("A"),
            "Expected 'A' in lossy output: {}",
            alt_debug
        );
    }

    #[test]
    fn exec_output_normal_debug_with_empty_fields() {
        let output = ExecOutput {
            success: true,
            stdout: vec![],
            stderr: vec![],
        };
        let debug_str = format!("{:?}", output);
        assert!(
            debug_str.contains("success: true"),
            "Expected 'success: true' in '{debug_str}'"
        );
        assert!(
            debug_str.contains("stdout: []"),
            "Expected empty stdout in '{debug_str}'"
        );
    }

    #[test]
    fn exec_output_alternate_debug_with_empty_fields() {
        let output = ExecOutput {
            success: false,
            stdout: vec![],
            stderr: vec![],
        };
        let alt_debug = format!("{:#?}", output);
        assert!(
            alt_debug.contains("success: false"),
            "Expected 'success: false' in '{alt_debug}'"
        );
    }

    #[test]
    fn exec_output_alternate_debug_with_newlines_in_output() {
        let output = ExecOutput {
            success: true,
            stdout: b"line1\nline2\nline3".to_vec(),
            stderr: b"err1\nerr2".to_vec(),
        };
        let alt_debug = format!("{:#?}", output);
        assert!(
            alt_debug.contains("line1"),
            "Expected line1 in '{alt_debug}'"
        );
    }

    #[test]
    fn exec_output_debug_success_true() {
        let output = ExecOutput {
            success: true,
            stdout: b"ok".to_vec(),
            stderr: vec![],
        };
        let debug = format!("{:?}", output);
        assert!(
            debug.contains("success: true"),
            "Expected success: true in '{debug}'"
        );
    }

    // --- ExecOutput equality, clone, and construction tests ---

    #[test]
    fn exec_output_equality() {
        let a = ExecOutput {
            success: true,
            stdout: b"hello".to_vec(),
            stderr: b"world".to_vec(),
        };
        let b = ExecOutput {
            success: true,
            stdout: b"hello".to_vec(),
            stderr: b"world".to_vec(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn exec_output_inequality_success() {
        let a = ExecOutput {
            success: true,
            stdout: vec![],
            stderr: vec![],
        };
        let b = ExecOutput {
            success: false,
            stdout: vec![],
            stderr: vec![],
        };
        assert_ne!(a, b);
    }

    #[test]
    fn exec_output_inequality_stdout() {
        let a = ExecOutput {
            success: true,
            stdout: b"abc".to_vec(),
            stderr: vec![],
        };
        let b = ExecOutput {
            success: true,
            stdout: b"xyz".to_vec(),
            stderr: vec![],
        };
        assert_ne!(a, b);
    }

    #[test]
    fn exec_output_inequality_stderr() {
        let a = ExecOutput {
            success: true,
            stdout: vec![],
            stderr: b"err1".to_vec(),
        };
        let b = ExecOutput {
            success: true,
            stdout: vec![],
            stderr: b"err2".to_vec(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn exec_output_clone() {
        let original = ExecOutput {
            success: true,
            stdout: b"output data".to_vec(),
            stderr: b"error data".to_vec(),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
        assert!(cloned.success);
        assert_eq!(cloned.stdout, b"output data");
        assert_eq!(cloned.stderr, b"error data");
    }

    #[test]
    fn exec_output_empty_fields() {
        let output = ExecOutput {
            success: false,
            stdout: vec![],
            stderr: vec![],
        };
        assert!(!output.success);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn exec_output_large_data() {
        let big_stdout = vec![0x41u8; 10_000]; // 10KB of 'A'
        let output = ExecOutput {
            success: true,
            stdout: big_stdout.clone(),
            stderr: vec![],
        };
        assert_eq!(output.stdout.len(), 10_000);
        assert_eq!(output.stdout, big_stdout);
    }

    #[test]
    fn exec_output_alternate_debug_shows_lossy_strings() {
        let output = ExecOutput {
            success: true,
            stdout: b"readable text".to_vec(),
            stderr: b"error text".to_vec(),
        };
        let alt_debug = format!("{:#?}", output);
        assert!(
            alt_debug.contains("readable text"),
            "Expected string stdout in alternate debug: {}",
            alt_debug
        );
        assert!(
            alt_debug.contains("error text"),
            "Expected string stderr in alternate debug: {}",
            alt_debug
        );
    }

    #[test]
    fn exec_output_normal_debug_shows_bytes() {
        let output = ExecOutput {
            success: true,
            stdout: vec![72, 105], // "Hi"
            stderr: vec![],
        };
        let normal_debug = format!("{:?}", output);
        assert!(
            normal_debug.contains("72") && normal_debug.contains("105"),
            "Expected byte values in normal debug: {}",
            normal_debug
        );
    }

    #[test]
    fn exec_output_self_equality() {
        let a = ExecOutput {
            success: true,
            stdout: b"data".to_vec(),
            stderr: b"err".to_vec(),
        };
        assert_eq!(a, a);
    }

    #[test]
    fn exec_output_clone_independence() {
        let original = ExecOutput {
            success: true,
            stdout: b"original".to_vec(),
            stderr: vec![],
        };
        let mut cloned = original.clone();
        cloned.stdout = b"modified".to_vec();
        cloned.success = false;

        // Original should be unaffected
        assert!(original.success);
        assert_eq!(original.stdout, b"original");
        assert!(!cloned.success);
        assert_eq!(cloned.stdout, b"modified");
    }

    #[test]
    fn exec_output_binary_data() {
        let output = ExecOutput {
            success: true,
            stdout: vec![0x00, 0x01, 0x02, 0xff, 0xfe],
            stderr: vec![0xde, 0xad, 0xbe, 0xef],
        };
        assert_eq!(output.stdout.len(), 5);
        assert_eq!(output.stderr.len(), 4);
    }

    #[test]
    fn exec_output_success_false_with_data() {
        let output = ExecOutput {
            success: false,
            stdout: b"partial output".to_vec(),
            stderr: b"command failed with exit code 1".to_vec(),
        };
        assert!(!output.success);
        assert!(!output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }

    // --- Constants verification ---

    #[test]
    fn ssh_exec_timeout_is_30_seconds() {
        assert_eq!(SSH_EXEC_TIMEOUT, Some(Duration::from_secs(30)));
    }

    // --- contains_subslice logic tests ---
    // Replicate the contains_subslice function from is_windows for testing

    fn contains_subslice(slice: &[u8], subslice: &[u8]) -> bool {
        if subslice.is_empty() {
            return true;
        }
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

    #[test]
    fn contains_subslice_finds_at_start() {
        assert!(contains_subslice(b"Windows_NT", b"Windows"));
    }

    #[test]
    fn contains_subslice_finds_at_end() {
        assert!(contains_subslice(b"Some Windows_NT", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_finds_in_middle() {
        assert!(contains_subslice(b"xxWindows_NTxx", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_not_found() {
        assert!(!contains_subslice(b"Linux", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_empty_subslice() {
        // Empty subslice is always found (starts_with empty is true)
        assert!(contains_subslice(b"anything", b""));
    }

    #[test]
    fn contains_subslice_empty_slice() {
        assert!(!contains_subslice(b"", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_both_empty() {
        // Empty subslice is contained in everything, matching [].starts_with(&[])
        assert!(contains_subslice(b"", b""));
    }

    #[test]
    fn contains_subslice_exact_match() {
        assert!(contains_subslice(b"Windows_NT", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_subslice_longer_than_slice() {
        assert!(!contains_subslice(b"Win", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_single_byte_found() {
        assert!(contains_subslice(b"abc", b"b"));
    }

    #[test]
    fn contains_subslice_single_byte_not_found() {
        assert!(!contains_subslice(b"abc", b"d"));
    }

    #[test]
    fn contains_subslice_repeated_pattern() {
        assert!(contains_subslice(b"ababab", b"bab"));
    }

    #[test]
    fn contains_subslice_partial_match_then_full() {
        assert!(contains_subslice(b"WinWindows_NT", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_binary_data() {
        assert!(contains_subslice(
            &[0x00, 0x01, 0xFF, 0xFE, 0x03],
            &[0xFF, 0xFE]
        ));
    }

    #[test]
    fn contains_subslice_binary_data_not_found() {
        assert!(!contains_subslice(&[0x00, 0x01, 0x02, 0x03], &[0xFF, 0xFE]));
    }

    #[test]
    fn contains_subslice_with_newlines() {
        assert!(contains_subslice(b"line1\r\nWindows_NT\r\n", b"Windows_NT"));
    }

    #[test]
    fn contains_subslice_with_whitespace_prefix() {
        assert!(contains_subslice(b"  \t Windows_NT  ", b"Windows_NT"));
    }

    // --- ExecOutput additional tests ---

    #[test]
    fn exec_output_alternate_debug_with_unicode() {
        let output = ExecOutput {
            success: true,
            stdout: "unicode content".as_bytes().to_vec(),
            stderr: "error message".as_bytes().to_vec(),
        };
        let alt_debug = format!("{:#?}", output);
        assert!(
            alt_debug.contains("unicode content"),
            "Expected unicode content in '{alt_debug}'"
        );
    }

    #[test]
    fn exec_output_normal_debug_contains_struct_name() {
        let output = ExecOutput {
            success: true,
            stdout: vec![],
            stderr: vec![],
        };
        let debug = format!("{:?}", output);
        assert!(
            debug.contains("ExecOutput"),
            "Expected struct name in '{debug}'"
        );
    }

    #[test]
    fn exec_output_clone_eq_reflexive() {
        let a = ExecOutput {
            success: false,
            stdout: b"test".to_vec(),
            stderr: b"err".to_vec(),
        };
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(b, a); // symmetric
    }

    #[test]
    fn exec_output_eq_transitive() {
        let a = ExecOutput {
            success: true,
            stdout: b"x".to_vec(),
            stderr: vec![],
        };
        let b = a.clone();
        let c = b.clone();
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a, c); // transitive
    }

    // --- SftpPathBuf tests ---

    use distant_core::protocol::RemotePath;

    use super::SftpPathBuf;
    use crate::SshFamily;

    // -- from_remote tests --

    #[test]
    fn from_remote_unix_absolute() {
        let p = SftpPathBuf::from_remote(&RemotePath::new("/home/user"), SshFamily::Unix);
        assert_eq!(p.as_str(), "/home/user");
    }

    #[test]
    fn from_remote_unix_relative() {
        let p = SftpPathBuf::from_remote(&RemotePath::new("relative/path"), SshFamily::Unix);
        assert_eq!(p.as_str(), "relative/path");
    }

    #[test]
    fn from_remote_windows_drive_letter() {
        let p = SftpPathBuf::from_remote(&RemotePath::new("C:\\Users\\foo"), SshFamily::Windows);
        assert_eq!(p.as_str(), "/C:/Users/foo");
    }

    #[test]
    fn from_remote_windows_relative() {
        let p = SftpPathBuf::from_remote(&RemotePath::new("sub1\\file2"), SshFamily::Windows);
        assert_eq!(p.as_str(), "sub1/file2");
    }

    #[test]
    fn from_remote_windows_forward_slash_drive() {
        let p = SftpPathBuf::from_remote(&RemotePath::new("C:/Users/foo"), SshFamily::Windows);
        assert_eq!(p.as_str(), "/C:/Users/foo");
    }

    // -- from_sftp + to_remote_path round-trip tests --

    #[test]
    fn to_remote_path_unix_passthrough() {
        let p = SftpPathBuf::from_sftp("/home/user", SshFamily::Unix);
        assert_eq!(p.to_remote_path(), RemotePath::new("/home/user"));
    }

    #[test]
    fn to_remote_path_windows_absolute_with_leading_slash() {
        let p = SftpPathBuf::from_sftp("/C:/Users/foo", SshFamily::Windows);
        assert_eq!(p.to_remote_path(), RemotePath::new("C:\\Users\\foo"));
    }

    #[test]
    fn to_remote_path_windows_absolute_without_leading_slash() {
        let p = SftpPathBuf::from_sftp("C:/Users/foo", SshFamily::Windows);
        assert_eq!(p.to_remote_path(), RemotePath::new("C:\\Users\\foo"));
    }

    #[test]
    fn to_remote_path_windows_relative() {
        let p = SftpPathBuf::from_sftp("sub1/file2", SshFamily::Windows);
        assert_eq!(p.to_remote_path(), RemotePath::new("sub1\\file2"));
    }

    #[test]
    fn from_remote_round_trip_unix() {
        let orig = RemotePath::new("/home/user/file.txt");
        let sftp = SftpPathBuf::from_remote(&orig, SshFamily::Unix);
        assert_eq!(sftp.to_remote_path(), orig);
    }

    #[test]
    fn from_remote_round_trip_windows() {
        let orig = RemotePath::new("C:\\Users\\foo\\bar.txt");
        let sftp = SftpPathBuf::from_remote(&orig, SshFamily::Windows);
        assert_eq!(sftp.to_remote_path(), orig);
    }

    // -- join tests --

    #[test]
    fn join_uses_forward_slash() {
        let base = SftpPathBuf::from_sftp("/home/user", SshFamily::Unix);
        let joined = base.join("file.txt");
        assert_eq!(joined.as_str(), "/home/user/file.txt");
    }

    #[test]
    fn join_empty_base() {
        let base = SftpPathBuf::from_sftp("", SshFamily::Unix);
        let joined = base.join("file.txt");
        assert_eq!(joined.as_str(), "file.txt");
    }

    #[test]
    fn join_trailing_separator() {
        let base = SftpPathBuf::from_sftp("/home/user/", SshFamily::Unix);
        let joined = base.join("file.txt");
        assert_eq!(joined.as_str(), "/home/user/file.txt");
    }

    #[test]
    fn join_windows_then_to_remote() {
        let base = SftpPathBuf::from_sftp("/C:/Users", SshFamily::Windows);
        let joined = base.join("foo");
        assert_eq!(joined.as_str(), "/C:/Users/foo");
        assert_eq!(joined.to_remote_path(), RemotePath::new("C:\\Users\\foo"));
    }

    #[test]
    fn join_relative_windows_then_to_remote() {
        let base = SftpPathBuf::from_sftp("sub1", SshFamily::Windows);
        let joined = base.join("file2");
        assert_eq!(joined.as_str(), "sub1/file2");
        assert_eq!(joined.to_remote_path(), RemotePath::new("sub1\\file2"));
    }

    #[test]
    fn join_relative_unix_then_to_remote() {
        let base = SftpPathBuf::from_sftp("sub1", SshFamily::Unix);
        let joined = base.join("file2");
        assert_eq!(joined.as_str(), "sub1/file2");
        assert_eq!(joined.to_remote_path(), RemotePath::new("sub1/file2"));
    }

    // -- file_name tests --

    #[test]
    fn file_name_unix_separator() {
        let p = SftpPathBuf::from_sftp("/home/user/file.txt", SshFamily::Unix);
        assert_eq!(p.file_name(), Some("file.txt"));
    }

    #[test]
    fn file_name_windows_separator() {
        let p = SftpPathBuf::from_sftp("C:\\Users\\foo\\bar.txt", SshFamily::Windows);
        assert_eq!(p.file_name(), Some("bar.txt"));
    }

    #[test]
    fn file_name_no_separator() {
        let p = SftpPathBuf::from_sftp("file.txt", SshFamily::Unix);
        assert_eq!(p.file_name(), Some("file.txt"));
    }

    #[test]
    fn file_name_trailing_slash() {
        let p = SftpPathBuf::from_sftp("/home/user/", SshFamily::Unix);
        assert_eq!(p.file_name(), Some("user"));
    }

    // -- strip_prefix tests --

    #[test]
    fn strip_prefix_basic() {
        let path = SftpPathBuf::from_sftp("/home/user/file.txt", SshFamily::Unix);
        let prefix = SftpPathBuf::from_sftp("/home/user", SshFamily::Unix);
        assert_eq!(path.strip_prefix(&prefix), Some("file.txt".to_string()));
    }

    #[test]
    fn strip_prefix_no_match() {
        let path = SftpPathBuf::from_sftp("/other/path", SshFamily::Unix);
        let prefix = SftpPathBuf::from_sftp("/home/user", SshFamily::Unix);
        assert_eq!(path.strip_prefix(&prefix), None);
    }

    #[test]
    fn strip_prefix_trailing_separator() {
        let path = SftpPathBuf::from_sftp("/home/user/file.txt", SshFamily::Unix);
        let prefix = SftpPathBuf::from_sftp("/home/user/", SshFamily::Unix);
        assert_eq!(path.strip_prefix(&prefix), Some("file.txt".to_string()));
    }

    // -- Display and From tests --

    #[test]
    fn display_shows_sftp_format() {
        let p = SftpPathBuf::from_sftp("/C:/Users/foo", SshFamily::Windows);
        assert_eq!(format!("{p}"), "/C:/Users/foo");
    }

    #[test]
    fn into_string_returns_inner() {
        let p = SftpPathBuf::from_sftp("/home/user", SshFamily::Unix);
        let s: String = p.into();
        assert_eq!(s, "/home/user");
    }
}
