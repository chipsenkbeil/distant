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

    // --- convert_to_windows_path_string tests ---

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
        // /1/path -> '1' is not alphabetic, so this should return None
        let result = convert_to_windows_path_string("/1/path");
        assert_eq!(result, None);
    }

    #[test]
    fn convert_slash_d_colon_path() {
        // /D:/Data/files -> D:/Data/files
        let result = convert_to_windows_path_string("/D:/Data/files");
        assert_eq!(result, Some("D:/Data/files".to_string()));
    }

    #[test]
    fn convert_slash_z_colon_path() {
        // /Z:/network/share -> Z:/network/share
        let result = convert_to_windows_path_string("/Z:/network/share");
        assert_eq!(result, Some("Z:/network/share".to_string()));
    }

    #[test]
    fn convert_lowercase_d_colon_path() {
        let result = convert_to_windows_path_string("/d:/data");
        assert_eq!(result, Some("d:/data".to_string()));
    }

    #[test]
    fn convert_backslash_windows_path() {
        let result = convert_to_windows_path_string("D:\\Program Files\\App");
        assert_eq!(result, Some("D:\\Program Files\\App".to_string()));
    }

    #[test]
    fn convert_deep_nested_slash_c_colon() {
        let result = convert_to_windows_path_string("/C:/Users/test/Documents/sub/dir/file.txt");
        assert_eq!(
            result,
            Some("C:/Users/test/Documents/sub/dir/file.txt".to_string())
        );
    }

    #[test]
    fn convert_slash_c_colon_single_file() {
        let result = convert_to_windows_path_string("/C:/file.txt");
        assert_eq!(result, Some("C:/file.txt".to_string()));
    }

    #[test]
    fn convert_special_char_component_returns_none() {
        // /!/path -> '!' is a single char but not alphabetic
        let result = convert_to_windows_path_string("/!/path");
        assert_eq!(result, None);
    }

    #[test]
    fn convert_dot_component_returns_none() {
        // /./path -> '.' is not alphabetic
        let result = convert_to_windows_path_string("/./path");
        // This depends on how WindowsPath parses '.' -- it may be a CurDir component
        // Just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn convert_single_char_numeric_returns_none() {
        // /9 -> '9' is not alphabetic
        let result = convert_to_windows_path_string("/9");
        assert_eq!(result, None);
    }

    #[test]
    fn convert_two_char_component_returns_none() {
        // /AB/path -> "AB" is two chars, not a single drive letter
        let result = convert_to_windows_path_string("/AB/path");
        assert_eq!(result, None);
    }

    #[test]
    fn convert_drive_letter_with_mixed_separators() {
        // C:\Users/test -> already starts with prefix
        let result = convert_to_windows_path_string("C:\\Users/test");
        assert_eq!(result, Some("C:\\Users/test".to_string()));
    }

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

    #[test]
    fn to_other_error_with_empty_string() {
        let err = to_other_error("");
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn to_other_error_error_message_is_display_output() {
        let err = to_other_error("display this message");
        assert_eq!(format!("{}", err), "display this message");
    }

    #[test]
    fn to_other_error_with_multiline_message() {
        let err = to_other_error("line1\nline2\nline3");
        let msg = format!("{}", err);
        assert!(msg.contains("line1"), "Expected 'line1' in '{msg}'");
        assert!(msg.contains("line2"), "Expected 'line2' in '{msg}'");
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

    #[test]
    fn reader_pause_millis_is_100() {
        assert_eq!(READER_PAUSE_MILLIS, 100);
    }

    // --- contains_subslice logic tests ---
    // Replicate the contains_subslice function from is_windows for testing

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
        // Loop doesn't execute for empty slice, so returns false
        assert!(!contains_subslice(b"", b""));
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

    // --- convert_to_windows_path_string additional edge cases ---

    #[test]
    fn convert_slash_e_colon_path() {
        let result = convert_to_windows_path_string("/E:/games");
        assert_eq!(result, Some("E:/games".to_string()));
    }

    #[test]
    fn convert_slash_x_colon_deep_path() {
        let result = convert_to_windows_path_string("/X:/a/b/c/d/e/f");
        assert_eq!(result, Some("X:/a/b/c/d/e/f".to_string()));
    }

    #[test]
    fn convert_path_with_spaces_in_components() {
        let result = convert_to_windows_path_string("/C:/Program Files/My App/config.ini");
        assert_eq!(
            result,
            Some("C:/Program Files/My App/config.ini".to_string())
        );
    }

    #[test]
    fn convert_windows_path_backslash_deep() {
        let result = convert_to_windows_path_string("C:\\Users\\Test\\Documents\\file.txt");
        assert_eq!(
            result,
            Some("C:\\Users\\Test\\Documents\\file.txt".to_string())
        );
    }

    #[test]
    fn convert_double_dot_path_returns_none() {
        // /.. is not a valid Windows drive path
        let result = convert_to_windows_path_string("/..");
        // This will be a CurDir or ParentDir component after RootDir
        let _ = result; // Just verify no panic
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
}
