use distant_ssh2::{Ssh, SshFamily};
use rstest::*;
use test_log::test;

use crate::sshd::*;

#[rstest]
#[test(tokio::test)]
async fn detect_family_should_return_windows_if_sshd_on_windows(#[future] ssh: Ctx<Ssh>) {
    let ssh = ssh.await;
    let family = ssh.detect_family().await.expect("Failed to detect family");

    // NOTE: We are testing against the local machine, so if Rust was compiled for Windows, then we
    //       are also on a Windows machine remotely for this test!
    assert_eq!(
        family,
        if cfg!(windows) {
            SshFamily::Windows
        } else {
            SshFamily::Unix
        },
        "Got wrong family"
    );
}
