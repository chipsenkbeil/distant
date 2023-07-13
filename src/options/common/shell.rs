use derive_more::{Display, Error};
use std::str::FromStr;
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

            // TODO: Powershell does not work because our splitting logic for arguments on
            //       distant-local does not handle single quotes. In fact, the splitting logic
            //       isn't designed for powershell at all. We need distant-local to detect that the
            //       command is powershell and alter parsing to something that works to split a
            //       string into the command and arguments. How do we do that?
            ShellKind::PowerShell => Ok(format!("{path} -Command '{}'", cmd.replace('\'', "''"))),

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
