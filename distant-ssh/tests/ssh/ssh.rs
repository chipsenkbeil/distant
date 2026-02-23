use std::time::Duration;

use assert_fs::prelude::*;
use distant_core::ChannelExt;
use distant_ssh::{LaunchOpts, Ssh, SshFamily, SshOpts};
use rstest::*;
use test_log::test;

use distant_test_harness::sshd::*;

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

#[rstest]
#[test(tokio::test)]
async fn detect_family_should_return_same_result_on_repeated_calls(#[future] ssh: Ctx<Ssh>) {
    let ssh = ssh.await;
    let family1 = ssh.detect_family().await.expect("First call failed");
    let family2 = ssh.detect_family().await.expect("Second call failed");
    assert_eq!(family1, family2, "Cached family should match");
}

#[rstest]
#[test(tokio::test)]
async fn is_authenticated_should_be_true_after_connect(#[future] ssh: Ctx<Ssh>) {
    let ssh = ssh.await;
    assert!(ssh.is_authenticated());
}

#[rstest]
#[test(tokio::test)]
async fn into_distant_pair_should_return_client_and_server(sshd: Sshd) {
    let ssh = load_ssh_client(&sshd).await;
    let (mut client, _server_ref) = ssh.into_distant_pair().await.unwrap();
    client.shutdown_on_drop(true);
    let info = client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
}

#[test(tokio::test)]
async fn connect_should_fail_on_refused_port() {
    let opts = SshOpts {
        port: Some(1),
        ..Default::default()
    };
    let result = Ssh::connect("127.0.0.1", opts).await;
    assert!(result.is_err());
}

#[rstest]
#[test(tokio::test)]
async fn connect_with_verbose_should_succeed(sshd: Sshd) {
    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![sshd.tmp.child("id_ed25519").path().to_path_buf()],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        verbose: true,
        ..Default::default()
    };
    let mut ssh = Ssh::connect("127.0.0.1", opts).await.unwrap();
    ssh.authenticate(MockSshAuthHandler).await.unwrap();
    assert!(ssh.is_authenticated());
}

#[rstest]
#[test(tokio::test)]
async fn ssh_host_and_port_accessors(#[future] ssh: Ctx<Ssh>) {
    let ssh = ssh.await;
    // Host should be some variant of localhost (127.0.0.1 or ::1)
    let host = ssh.host();
    assert!(
        host == "127.0.0.1" || host == "::1",
        "Unexpected host: {}",
        host
    );
    assert_eq!(ssh.port(), ssh.sshd.port);
}

#[rstest]
#[test(tokio::test)]
async fn authenticate_twice_should_succeed(sshd: Sshd) {
    let mut ssh = load_ssh_client(&sshd).await;
    // Already authenticated by load_ssh_client, call again should be a no-op
    assert!(ssh.is_authenticated());
    ssh.authenticate(MockSshAuthHandler).await.unwrap();
    assert!(ssh.is_authenticated());
}

#[rstest]
#[test(tokio::test)]
async fn into_distant_pair_server_ref_is_alive(sshd: Sshd) {
    let ssh = load_ssh_client(&sshd).await;
    let (mut client, server_ref) = ssh.into_distant_pair().await.unwrap();
    client.shutdown_on_drop(true);
    assert!(!server_ref.is_finished(), "Server should be running");
    let _ = client.system_info().await.unwrap();
    assert!(
        !server_ref.is_finished(),
        "Server should still be running after request"
    );
}

#[rstest]
#[test(tokio::test)]
async fn launch_with_nonexistent_binary_should_fail(sshd: Sshd) {
    let ssh = load_ssh_client(&sshd).await;
    let opts = LaunchOpts {
        binary: String::from("nonexistent_distant_binary_xyz_12345"),
        args: String::new(),
        timeout: Duration::from_secs(3),
    };
    let result = ssh.launch(opts).await;
    assert!(
        result.is_err(),
        "Launch with nonexistent binary should fail"
    );
}

#[rstest]
#[test(tokio::test)]
async fn launch_and_connect_should_return_working_client(sshd: Sshd) {
    let ssh = load_ssh_client(&sshd).await;
    let opts = LaunchOpts {
        binary: String::from("distant"),
        args: String::new(),
        timeout: Duration::from_secs(15),
    };
    let mut client = ssh.launch_and_connect(opts).await.unwrap();
    let info = client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
    let _ = client.shutdown().await;
}

#[test(tokio::test)]
async fn connect_failure_error_should_be_connection_refused() {
    let opts = SshOpts {
        port: Some(1),
        ..Default::default()
    };
    let result = Ssh::connect("127.0.0.1", opts).await;
    match result {
        Err(err) => {
            // Should be a connection error
            assert!(
                err.kind() == std::io::ErrorKind::ConnectionRefused
                    || err.kind() == std::io::ErrorKind::Other,
                "Unexpected error kind: {:?} - {}",
                err.kind(),
                err
            );
        }
        Ok(_) => panic!("Expected connection to fail"),
    }
}
