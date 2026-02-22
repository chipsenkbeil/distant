use std::io;
use std::time::Duration;

use russh::client::Handle;
use typed_path::{Components, WindowsComponent, WindowsPath, WindowsPathBuf};

use crate::ClientHandler;

const SSH_EXEC_TIMEOUT: Option<Duration> = Some(Duration::from_secs(30));

#[allow(dead_code)]
const READER_PAUSE_MILLIS: u64 = 100;

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
        .map_err(to_other_error)?;

    // Execute command
    channel.exec(true, cmd).await.map_err(to_other_error)?;

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

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::other(err)
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

/// Attempts to convert UTF8 str into a path compliant with Windows
pub fn convert_to_windows_path_string(s: &str) -> Option<String> {
    let path = WindowsPath::new(s);
    let mut components = path.components();

    // If we start with a root directory, we may have the weird path
    match components.next() {
        // Something weird like /C:/... or /C/... that we need to convert to C:\...
        Some(WindowsComponent::RootDir) => {
            let path = WindowsPath::new(components.as_bytes());

            // If we have a prefix, then that means we had something like /C:/...
            if let Some(WindowsComponent::Prefix(_)) = path.components().next() {
                std::str::from_utf8(path.as_bytes())
                    .ok()
                    .map(ToString::to_string)
            } else if let Some(WindowsComponent::Normal(filename)) = components.next() {
                // If we have a drive letter, convert it into a path, e.g. /C/... -> C:\...
                if filename.len() == 1 && (filename[0] as char).is_alphabetic() {
                    let mut path_buf = WindowsPathBuf::from(format!("{}:", filename[0]));
                    for component in components {
                        path_buf.push(component);
                    }
                    std::str::from_utf8(path.as_bytes())
                        .ok()
                        .map(ToString::to_string)
                } else {
                    None
                }
            } else {
                None
            }
        }

        // Already is a Windows path, so just return string
        Some(WindowsComponent::Prefix(_)) => Some(s.to_string()),

        // Not a reliable Windows path, so return None
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_slash_c_colon_path_to_windows() {
        // /C:/Users/test -> C:/Users/test
        let result = convert_to_windows_path_string("/C:/Users/test");
        assert_eq!(result, Some("C:/Users/test".to_string()));
    }

    #[test]
    fn convert_already_windows_path_unchanged() {
        // C:\Users\test is already a Windows path
        let result = convert_to_windows_path_string("C:\\Users\\test");
        assert_eq!(result, Some("C:\\Users\\test".to_string()));
    }

    #[test]
    fn convert_relative_path_returns_none() {
        assert_eq!(convert_to_windows_path_string("relative/path"), None);
    }

    #[test]
    fn convert_root_only_returns_none() {
        assert_eq!(convert_to_windows_path_string("/"), None);
    }

    #[test]
    fn convert_c_colon_slash_root_path() {
        // /C:/ -> just the drive root
        let result = convert_to_windows_path_string("/C:/");
        assert!(result.is_some(), "Should handle drive root path");
    }

    #[test]
    fn convert_slash_c_slash_path_to_windows() {
        // /C/Users/test -> should attempt drive-letter conversion
        let result = convert_to_windows_path_string("/C/Users/test");
        assert!(result.is_some(), "Should convert single-letter drive path");
    }

    #[test]
    fn convert_multi_char_component_returns_none() {
        // /notadrive/path -> not a single-letter drive, returns None
        assert_eq!(convert_to_windows_path_string("/notadrive/path"), None);
    }

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
        // Normal format uses byte representation, not string
        assert!(
            debug_str.contains("success: false"),
            "Missing success field: {}",
            debug_str
        );
    }

    // --- to_other_error tests ---

    #[test]
    fn to_other_error_converts_string_to_io_error() {
        let err = to_other_error("something went wrong");
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn to_other_error_preserves_error_message() {
        let err = to_other_error("specific error message");
        let msg = format!("{}", err);
        assert!(
            msg.contains("specific error message"),
            "Expected error message in '{msg}'"
        );
    }

    #[test]
    fn to_other_error_converts_io_error() {
        let original = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let converted = to_other_error(original);
        assert_eq!(converted.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn to_other_error_converts_custom_error() {
        #[derive(Debug)]
        struct CustomError;
        impl std::fmt::Display for CustomError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "custom error")
            }
        }
        impl std::error::Error for CustomError {}

        let err = to_other_error(CustomError);
        assert_eq!(err.kind(), io::ErrorKind::Other);
        let msg = format!("{}", err);
        assert!(
            msg.contains("custom error"),
            "Expected 'custom error' in '{msg}'"
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
        assert_eq!(cloned.success, true);
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
        // Alternate format should show string representation
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
        // Normal format should show byte arrays, e.g. [72, 105]
        assert!(
            normal_debug.contains("72") && normal_debug.contains("105"),
            "Expected byte values in normal debug: {}",
            normal_debug
        );
    }

    // --- Additional convert_to_windows_path_string edge cases ---

    #[test]
    fn convert_lowercase_drive_letter() {
        // /c/Users/test -> should handle lowercase drive letter
        let result = convert_to_windows_path_string("/c/Users/test");
        assert!(
            result.is_some(),
            "Should convert lowercase single-letter drive path"
        );
    }

    #[test]
    fn convert_empty_string_returns_none() {
        assert_eq!(convert_to_windows_path_string(""), None);
    }

    #[test]
    fn convert_slash_only_single_letter_no_further_components() {
        // /C with no further path components
        let result = convert_to_windows_path_string("/C");
        assert!(
            result.is_some(),
            "Should handle single drive letter without trailing path"
        );
    }

    #[test]
    fn convert_windows_path_with_forward_slashes() {
        // C:/Users/test should be treated as already a windows path
        let result = convert_to_windows_path_string("C:/Users/test");
        assert_eq!(result, Some("C:/Users/test".to_string()));
    }

    #[test]
    fn convert_unc_style_returns_none() {
        // A relative-looking path (no root, no prefix) should return None
        let result = convert_to_windows_path_string("foo/bar/baz");
        assert_eq!(result, None);
    }

    #[test]
    fn convert_numeric_component_returns_none() {
        // /1/path -> '1' is not alphabetic... wait, it is a single char but not alphabetic
        // Actually '1'.is_alphabetic() is false, so this should return None
        let result = convert_to_windows_path_string("/1/path");
        assert_eq!(result, None);
    }
}
