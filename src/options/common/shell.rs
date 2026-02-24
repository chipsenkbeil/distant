use std::str::FromStr;

use derive_more::{Display, Error};
use typed_path::{Utf8UnixPath, Utf8WindowsPath};

/// Represents a shell to execute on the remote machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shell {
    /// Represents the path to the shell on the remote machine.
    pub path: String,

    /// Represents the kind of shell.
    pub kind: ShellKind,
}

impl Shell {
    #[inline]
    pub fn is_posix(&self) -> bool {
        self.kind.is_posix()
    }

    /// Wraps a `cmd` such that it is invoked by this shell.
    ///
    /// * For `cmd.exe`, this wraps in double quotes such that it can be invoked by `cmd.exe /S /C "..."`.
    /// * For `powershell.exe`, this wraps in single quotes and escapes single quotes by doubling
    ///   them such that it can be invoked by `powershell.exe -Command '...'`.
    /// * For `rc` and `elvish`, this wraps in single quotes and escapes single quotes by doubling them.
    ///     * For rc and elvish, this uses `shell -c '...'`.
    /// * For **POSIX** shells, this wraps in single quotes and uses the trick of `'\''` to fake escape.
    /// * For `nu`, this wraps in single quotes or backticks where possible, but fails if the cmd contains single quotes and backticks.
    ///
    pub fn make_cmd_string(&self, cmd: &str) -> Result<String, &'static str> {
        let path = self.path.as_str();

        match self.kind {
            ShellKind::CmdExe => Ok(format!("{path} /S /C \"{cmd}\"")),

            // NOTE: Powershell does not work directly because our splitting logic for arguments on
            //       distant-host does not handle single quotes. In fact, the splitting logic
            //       isn't designed for powershell at all. To get around that limitation, we are
            //       using cmd.exe to invoke powershell, which fits closer to our parsing rules.
            //       Crazy, I know! Eventually, we should switch to properly using powershell
            //       and escaping single quotes by doubling them.
            ShellKind::PowerShell => Ok(format!(
                "cmd.exe /S /C \"{path} -Command {}\"",
                cmd.replace('"', "\"\""),
            )),

            ShellKind::Rc | ShellKind::Elvish => {
                Ok(format!("{path} -c '{}'", cmd.replace('\'', "''")))
            }

            ShellKind::Nu => {
                let has_single_quotes = cmd.contains('\'');
                let has_backticks = cmd.contains('`');

                match (has_single_quotes, has_backticks) {
                    // If we have both single quotes and backticks, fail
                    (true, true) => {
                        Err("unable to escape single quotes and backticks at the same time with nu")
                    }

                    // If we only have single quotes, use backticks
                    (true, false) => Ok(format!("{path} -c `{cmd}`")),

                    // Otherwise, we can safely use single quotes
                    _ => Ok(format!("{path} -c '{cmd}'")),
                }
            }

            // We assume anything else not specially handled is POSIX
            _ => Ok(format!("{path} -c '{}'", cmd.replace('\'', "'\\''"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Display, Error)]
pub struct ParseShellError(#[error(not(source))] String);

impl FromStr for Shell {
    type Err = ParseShellError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        let kind = ShellKind::identify(s)
            .ok_or_else(|| ParseShellError(format!("Unsupported shell: {s}")))?;

        Ok(Self {
            path: s.to_string(),
            kind,
        })
    }
}

/// Supported types of shells.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShellKind {
    Ash,
    Bash,
    CmdExe,
    Csh,
    Dash,
    Elvish,
    Fish,
    Ksh,
    Loksh,
    Mksh,
    Nu,
    Pdksh,
    PowerShell,
    Rc,
    Scsh,
    Sh,
    Tcsh,
    Zsh,
}

impl ShellKind {
    /// Returns true if shell represents a POSIX-compliant implementation.
    pub fn is_posix(&self) -> bool {
        matches!(
            self,
            Self::Ash
                | Self::Bash
                | Self::Csh
                | Self::Dash
                | Self::Fish
                | Self::Ksh
                | Self::Loksh
                | Self::Mksh
                | Self::Pdksh
                | Self::Scsh
                | Self::Sh
                | Self::Tcsh
                | Self::Zsh
        )
    }

    /// Identifies the shell kind from the given string. This string could be a Windows path, Unix
    /// path, or solo shell name.
    ///
    /// The process is handled by these steps:
    ///
    /// 1. Check if the string matches a shell name verbatim
    /// 2. Parse the path as a Unix path and check the file name for a match
    /// 3. Parse the path as a Windows path and check the file name for a match
    ///
    pub fn identify(s: &str) -> Option<Self> {
        Self::from_name(s)
            .or_else(|| Utf8UnixPath::new(s).file_name().and_then(Self::from_name))
            .or_else(|| {
                Utf8WindowsPath::new(s)
                    .file_name()
                    .and_then(Self::from_name)
            })
    }

    fn from_name(name: &str) -> Option<Self> {
        macro_rules! map_str {
            ($($name:literal -> $value:expr),+ $(,)?) => {{
                $(
                    if name.trim().eq_ignore_ascii_case($name) {
                        return Some($value);
                    }

                )+

                None
            }};
        }

        map_str! {
            "ash" -> Self::Ash,
            "bash" -> Self::Bash,
            "cmd" -> Self::CmdExe,
            "cmd.exe" -> Self::CmdExe,
            "csh" -> Self::Csh,
            "dash" -> Self::Dash,
            "elvish" -> Self::Elvish,
            "fish" -> Self::Fish,
            "ksh" -> Self::Ksh,
            "loksh" -> Self::Loksh,
            "mksh" -> Self::Mksh,
            "nu" -> Self::Nu,
            "pdksh" -> Self::Pdksh,
            "powershell" -> Self::PowerShell,
            "powershell.exe" -> Self::PowerShell,
            "rc" -> Self::Rc,
            "scsh" -> Self::Scsh,
            "sh" -> Self::Sh,
            "tcsh" -> Self::Tcsh,
            "zsh" -> Self::Zsh,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `ShellKind` identification (bare names, paths, case-insensitivity),
    //! `is_posix()`, `Shell` parsing, and `make_cmd_string()` for all five shell
    //! families (POSIX, cmd.exe, PowerShell, rc/elvish, nu).

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // ShellKind::identify – bare names
    // -------------------------------------------------------
    #[test]
    fn identify_bare_name_case_insensitive() {
        assert_eq!(ShellKind::identify("bash"), Some(ShellKind::Bash));
        assert_eq!(ShellKind::identify("BASH"), Some(ShellKind::Bash));
        assert_eq!(ShellKind::identify("Bash"), Some(ShellKind::Bash));
    }

    #[test]
    fn identify_all_known_bare_names() {
        let cases = vec![
            ("ash", ShellKind::Ash),
            ("bash", ShellKind::Bash),
            ("cmd", ShellKind::CmdExe),
            ("cmd.exe", ShellKind::CmdExe),
            ("csh", ShellKind::Csh),
            ("dash", ShellKind::Dash),
            ("elvish", ShellKind::Elvish),
            ("fish", ShellKind::Fish),
            ("ksh", ShellKind::Ksh),
            ("loksh", ShellKind::Loksh),
            ("mksh", ShellKind::Mksh),
            ("nu", ShellKind::Nu),
            ("pdksh", ShellKind::Pdksh),
            ("powershell", ShellKind::PowerShell),
            ("powershell.exe", ShellKind::PowerShell),
            ("rc", ShellKind::Rc),
            ("scsh", ShellKind::Scsh),
            ("sh", ShellKind::Sh),
            ("tcsh", ShellKind::Tcsh),
            ("zsh", ShellKind::Zsh),
        ];
        for (name, expected) in cases {
            assert_eq!(
                ShellKind::identify(name),
                Some(expected),
                "failed for bare name: {name}"
            );
        }
    }

    #[test]
    fn identify_returns_none_for_unknown_shell() {
        assert_eq!(ShellKind::identify("unknown_shell"), None);
        assert_eq!(ShellKind::identify(""), None);
    }

    // -------------------------------------------------------
    // ShellKind::identify – Unix paths
    // -------------------------------------------------------
    #[test]
    fn identify_unix_path() {
        assert_eq!(ShellKind::identify("/bin/bash"), Some(ShellKind::Bash));
        assert_eq!(ShellKind::identify("/usr/bin/zsh"), Some(ShellKind::Zsh));
        assert_eq!(
            ShellKind::identify("/usr/local/bin/fish"),
            Some(ShellKind::Fish)
        );
        assert_eq!(ShellKind::identify("/bin/sh"), Some(ShellKind::Sh));
    }

    // -------------------------------------------------------
    // ShellKind::identify – Windows paths
    // -------------------------------------------------------
    #[test]
    fn identify_windows_path() {
        assert_eq!(
            ShellKind::identify(r"C:\Windows\System32\cmd.exe"),
            Some(ShellKind::CmdExe)
        );
        assert_eq!(
            ShellKind::identify(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            Some(ShellKind::PowerShell)
        );
    }

    // -------------------------------------------------------
    // ShellKind::is_posix
    // -------------------------------------------------------
    #[test]
    fn is_posix_returns_true_for_posix_shells() {
        let posix = vec![
            ShellKind::Ash,
            ShellKind::Bash,
            ShellKind::Csh,
            ShellKind::Dash,
            ShellKind::Fish,
            ShellKind::Ksh,
            ShellKind::Loksh,
            ShellKind::Mksh,
            ShellKind::Pdksh,
            ShellKind::Scsh,
            ShellKind::Sh,
            ShellKind::Tcsh,
            ShellKind::Zsh,
        ];
        for kind in posix {
            assert!(kind.is_posix(), "{kind:?} should be POSIX");
        }
    }

    #[test]
    fn is_posix_returns_false_for_non_posix_shells() {
        let non_posix = vec![
            ShellKind::CmdExe,
            ShellKind::PowerShell,
            ShellKind::Rc,
            ShellKind::Elvish,
            ShellKind::Nu,
        ];
        for kind in non_posix {
            assert!(!kind.is_posix(), "{kind:?} should NOT be POSIX");
        }
    }

    // -------------------------------------------------------
    // Shell::from_str (parsing)
    // -------------------------------------------------------
    #[test]
    fn parse_shell_from_bare_name() {
        let shell: Shell = "bash".parse().unwrap();
        assert_eq!(shell.path, "bash");
        assert_eq!(shell.kind, ShellKind::Bash);
    }

    #[test]
    fn parse_shell_from_unix_path() {
        let shell: Shell = "/usr/bin/zsh".parse().unwrap();
        assert_eq!(shell.path, "/usr/bin/zsh");
        assert_eq!(shell.kind, ShellKind::Zsh);
    }

    #[test]
    fn parse_shell_from_windows_path() {
        let shell: Shell = r"C:\Windows\System32\cmd.exe".parse().unwrap();
        assert_eq!(shell.path, r"C:\Windows\System32\cmd.exe");
        assert_eq!(shell.kind, ShellKind::CmdExe);
    }

    #[test]
    fn parse_shell_trims_whitespace() {
        let shell: Shell = "  bash  ".parse().unwrap();
        assert_eq!(shell.path, "bash");
        assert_eq!(shell.kind, ShellKind::Bash);
    }

    #[test]
    fn parse_shell_error_for_unknown() {
        let result = "totally_unknown".parse::<Shell>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unsupported shell"));
    }

    // -------------------------------------------------------
    // Shell::is_posix delegates to ShellKind::is_posix
    // -------------------------------------------------------
    #[test]
    fn shell_is_posix_delegates_correctly() {
        let bash: Shell = "bash".parse().unwrap();
        assert!(bash.is_posix());

        let cmd: Shell = "cmd.exe".parse().unwrap();
        assert!(!cmd.is_posix());
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – cmd.exe
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_cmd_exe() {
        let shell: Shell = "cmd.exe".parse().unwrap();
        let result = shell.make_cmd_string("echo hello").unwrap();
        assert_eq!(result, r#"cmd.exe /S /C "echo hello""#);
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – powershell
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_powershell_basic() {
        let shell: Shell = "powershell.exe".parse().unwrap();
        let result = shell.make_cmd_string("Get-Process").unwrap();
        assert_eq!(
            result,
            r#"cmd.exe /S /C "powershell.exe -Command Get-Process""#
        );
    }

    #[test]
    fn make_cmd_string_powershell_escapes_double_quotes() {
        let shell: Shell = "powershell.exe".parse().unwrap();
        let result = shell.make_cmd_string(r#"echo "hi""#).unwrap();
        assert_eq!(
            result,
            r#"cmd.exe /S /C "powershell.exe -Command echo ""hi""""#
        );
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – rc / elvish (single-quote doubling)
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_rc_basic() {
        let shell: Shell = "rc".parse().unwrap();
        let result = shell.make_cmd_string("echo hello").unwrap();
        assert_eq!(result, "rc -c 'echo hello'");
    }

    #[test]
    fn make_cmd_string_elvish_escapes_single_quotes() {
        let shell: Shell = "elvish".parse().unwrap();
        let result = shell.make_cmd_string("echo 'world'").unwrap();
        assert_eq!(result, "elvish -c 'echo ''world'''");
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – nu
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_nu_no_special_chars() {
        let shell: Shell = "nu".parse().unwrap();
        let result = shell.make_cmd_string("echo hello").unwrap();
        assert_eq!(result, "nu -c 'echo hello'");
    }

    #[test]
    fn make_cmd_string_nu_with_single_quotes_uses_backticks() {
        let shell: Shell = "nu".parse().unwrap();
        let result = shell.make_cmd_string("echo 'hello'").unwrap();
        assert_eq!(result, "nu -c `echo 'hello'`");
    }

    #[test]
    fn make_cmd_string_nu_with_backticks_uses_single_quotes() {
        let shell: Shell = "nu".parse().unwrap();
        let result = shell.make_cmd_string("echo `hello`").unwrap();
        assert_eq!(result, "nu -c 'echo `hello`'");
    }

    #[test]
    fn make_cmd_string_nu_with_both_quotes_and_backticks_fails() {
        let shell: Shell = "nu".parse().unwrap();
        let result = shell.make_cmd_string("echo 'hello' `world`");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("unable to escape single quotes and backticks")
        );
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – POSIX shells (single-quote trick)
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_posix_basic() {
        let shell: Shell = "bash".parse().unwrap();
        let result = shell.make_cmd_string("echo hello").unwrap();
        assert_eq!(result, "bash -c 'echo hello'");
    }

    #[test]
    fn make_cmd_string_posix_escapes_single_quotes() {
        let shell: Shell = "/bin/sh".parse().unwrap();
        let result = shell.make_cmd_string("echo 'world'").unwrap();
        assert_eq!(result, r"/bin/sh -c 'echo '\''world'\'''");
    }

    #[test]
    fn make_cmd_string_posix_with_unix_path() {
        let shell: Shell = "/usr/bin/zsh".parse().unwrap();
        let result = shell.make_cmd_string("ls -la").unwrap();
        assert_eq!(result, "/usr/bin/zsh -c 'ls -la'");
    }

    // -------------------------------------------------------
    // Shell::make_cmd_string – windows path for cmd.exe
    // -------------------------------------------------------
    #[test]
    fn make_cmd_string_cmd_exe_windows_path() {
        let shell: Shell = r"C:\Windows\System32\cmd.exe".parse().unwrap();
        let result = shell.make_cmd_string("dir").unwrap();
        assert_eq!(result, r#"C:\Windows\System32\cmd.exe /S /C "dir""#);
    }
}
