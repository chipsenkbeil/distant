use crate::sshd::*;
use distant_ssh2::{Ssh, SshFamily};
use rstest::*;

#[cfg(unix)]
#[rstest]
#[tokio::test]
async fn detect_family_should_return_unix_if_sshd_on_unix(#[future] ssh: Ssh) {
    let ssh = ssh.await;
    let family = ssh.detect_family().await.expect("Failed to detect family");
    assert_eq!(family, SshFamily::Unix, "Got wrong family");
}

#[cfg(windows)]
#[rstest]
#[tokio::test]
async fn detect_family_should_return_windows_if_sshd_on_windows(#[future] ssh: Ssh) {
    let ssh = ssh.await;
    let family = ssh.detect_family().await.expect("Failed to detect family");
    assert_eq!(family, SshFamily::Windows, "Got wrong family");
}
