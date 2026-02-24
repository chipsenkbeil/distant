use std::ops::{Deref, DerefMut};

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};

/// Represents some command with arguments to execute
#[derive(Clone, Debug, Display, From, Into, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cmd(String);

impl Cmd {
    /// Creates a new command from the given `cmd`
    pub fn new(cmd: impl Into<String>) -> Self {
        Self(cmd.into())
    }
}

impl Deref for Cmd {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Cmd {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Cmd: construction, Deref/DerefMut to String, Display, From/Into conversions,
    //! and equality.

    use super::*;

    #[test]
    fn should_be_able_to_serialize_to_json() {
        let cmd = Cmd::new("echo some text");

        let value = serde_json::to_value(cmd).unwrap();
        assert_eq!(value, serde_json::json!("echo some text"));
    }

    #[test]
    fn should_be_able_to_deserialize_from_json() {
        let value = serde_json::json!("echo some text");

        let cmd: Cmd = serde_json::from_value(value).unwrap();
        assert_eq!(cmd, Cmd::new("echo some text"));
    }

    #[test]
    fn should_be_able_to_serialize_to_msgpack() {
        let cmd = Cmd::new("echo some text");

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&cmd).unwrap();
    }

    #[test]
    fn should_be_able_to_deserialize_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or causing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&Cmd::new("echo some text")).unwrap();

        let cmd: Cmd = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(cmd, Cmd::new("echo some text"));
    }

    #[test]
    fn new_should_accept_str() {
        let cmd = Cmd::new("ls -la");
        assert_eq!(&*cmd, "ls -la");
    }

    #[test]
    fn new_should_accept_string() {
        let cmd = Cmd::new(String::from("ls -la"));
        assert_eq!(&*cmd, "ls -la");
    }

    #[test]
    fn deref_should_provide_string_access() {
        let cmd = Cmd::new("echo hello");
        let s: &String = &cmd;
        assert_eq!(s, "echo hello");
    }

    #[test]
    fn deref_mut_should_provide_mutable_string_access() {
        let mut cmd = Cmd::new("echo");
        cmd.push_str(" world");
        assert_eq!(&*cmd, "echo world");
    }

    #[test]
    fn display_should_show_inner_string() {
        let cmd = Cmd::new("cat /etc/hosts");
        assert_eq!(cmd.to_string(), "cat /etc/hosts");
    }

    #[test]
    fn from_string_should_create_cmd() {
        let cmd: Cmd = String::from("pwd").into();
        assert_eq!(&*cmd, "pwd");
    }

    #[test]
    fn into_string_should_extract_inner() {
        let cmd = Cmd::new("whoami");
        let s: String = cmd.into();
        assert_eq!(s, "whoami");
    }

    #[test]
    fn equality_should_compare_inner_strings() {
        let a = Cmd::new("test");
        let b = Cmd::new("test");
        assert_eq!(a, b);
    }

    #[test]
    fn inequality_should_detect_different_strings() {
        let a = Cmd::new("foo");
        let b = Cmd::new("bar");
        assert_ne!(a, b);
    }
}
