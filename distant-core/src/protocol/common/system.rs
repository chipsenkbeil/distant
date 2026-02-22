use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Represents information about a system
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    /// Family of the operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.FAMILY.html
    pub family: String,

    /// Name of the specific operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.OS.html
    pub os: String,

    /// Architecture of the CPI as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.ARCH.html
    pub arch: String,

    /// Current working directory of the running server process
    pub current_dir: PathBuf,

    /// Primary separator for path components for the current platform
    /// as defined in https://doc.rust-lang.org/std/path/constant.MAIN_SEPARATOR.html
    pub main_separator: char,

    /// Name of the user running the server process
    pub username: String,

    /// Default shell tied to user running the server process
    pub shell: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_be_able_to_serialize_to_json() {
        let info = SystemInfo {
            family: String::from("family"),
            os: String::from("os"),
            arch: String::from("arch"),
            current_dir: PathBuf::from("current-dir"),
            main_separator: '/',
            username: String::from("username"),
            shell: String::from("shell"),
        };

        let value = serde_json::to_value(info).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "family": "family",
                "os": "os",
                "arch": "arch",
                "current_dir": "current-dir",
                "main_separator": '/',
                "username": "username",
                "shell": "shell",
            })
        );
    }

    #[test]
    fn should_be_able_to_deserialize_from_json() {
        let value = serde_json::json!({
            "family": "family",
            "os": "os",
            "arch": "arch",
            "current_dir": "current-dir",
            "main_separator": '/',
            "username": "username",
            "shell": "shell",
        });

        let info: SystemInfo = serde_json::from_value(value).unwrap();
        assert_eq!(
            info,
            SystemInfo {
                family: String::from("family"),
                os: String::from("os"),
                arch: String::from("arch"),
                current_dir: PathBuf::from("current-dir"),
                main_separator: '/',
                username: String::from("username"),
                shell: String::from("shell"),
            }
        );
    }

    #[test]
    fn should_be_able_to_serialize_to_msgpack() {
        let info = SystemInfo {
            family: String::from("family"),
            os: String::from("os"),
            arch: String::from("arch"),
            current_dir: PathBuf::from("current-dir"),
            main_separator: '/',
            username: String::from("username"),
            shell: String::from("shell"),
        };

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&info).unwrap();
    }

    #[test]
    fn should_be_able_to_deserialize_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or causing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&SystemInfo {
            family: String::from("family"),
            os: String::from("os"),
            arch: String::from("arch"),
            current_dir: PathBuf::from("current-dir"),
            main_separator: '/',
            username: String::from("username"),
            shell: String::from("shell"),
        })
        .unwrap();

        let info: SystemInfo = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            info,
            SystemInfo {
                family: String::from("family"),
                os: String::from("os"),
                arch: String::from("arch"),
                current_dir: PathBuf::from("current-dir"),
                main_separator: '/',
                username: String::from("username"),
                shell: String::from("shell"),
            }
        );
    }
}
