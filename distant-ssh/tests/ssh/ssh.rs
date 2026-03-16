use std::io;
use std::time::Duration;

use assert_fs::prelude::*;
use distant_core::ChannelExt;
use distant_ssh::{
    AuthResult, LaunchOpts, Ssh, SshAuthEvent, SshAuthHandler, SshFamily, SshOpts, SshSession,
};
use distant_test_harness::manager::bin_path;
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
    let result = SshSession::connect("127.0.0.1", opts).await;
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
    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    let ssh = match ssh.authenticate(MockSshAuthHandler).await {
        AuthResult::Authenticated(ssh) => ssh,
        AuthResult::Failed { error, .. } => panic!("Authentication failed: {error}"),
    };
    // Type-state ensures we are authenticated — verify we can use the client
    let _family = ssh.detect_family().await.unwrap();
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
        ..Default::default()
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
        binary: bin_path().to_string_lossy().to_string(),
        args: String::new(),
        timeout: Duration::from_secs(15),
        ..Default::default()
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
    let result = SshSession::connect("127.0.0.1", opts).await;
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

/// A custom auth handler that returns a passphrase when prompted for key decryption.
struct PassphraseSshAuthHandler {
    passphrase: String,
}

impl SshAuthHandler for PassphraseSshAuthHandler {
    async fn on_authenticate(&self, event: SshAuthEvent) -> io::Result<Vec<String>> {
        // Return the passphrase for any prompt
        Ok(vec![self.passphrase.clone(); event.prompts.len()])
    }

    async fn on_verify_host(&self, _host: &str) -> io::Result<bool> {
        Ok(true)
    }

    async fn on_banner(&self, _text: &str) {}

    async fn on_error(&self, _text: &str) {}
}

#[rstest]
#[test(tokio::test)]
async fn encrypted_key_should_authenticate_with_passphrase(sshd: Sshd) {
    let passphrase = "test_integration_passphrase";

    // Generate a passphrase-protected ed25519 key in a separate temp dir
    let key_dir = assert_fs::TempDir::new().unwrap();
    let encrypted_key_path = key_dir.child("id_ed25519_enc").path().to_path_buf();
    assert!(
        SshKeygen::generate_ed25519(&encrypted_key_path, passphrase)
            .expect("Failed to generate encrypted key"),
        "ssh-keygen failed to generate encrypted key"
    );

    // Add the public key to sshd's authorized_keys
    let pub_key_contents =
        std::fs::read_to_string(encrypted_key_path.with_extension("pub")).unwrap();
    let authorized_keys_path = sshd.tmp.child("authorized_keys").path().to_path_buf();
    let mut existing_keys = std::fs::read_to_string(&authorized_keys_path).unwrap_or_default();
    existing_keys.push('\n');
    existing_keys.push_str(&pub_key_contents);
    std::fs::write(&authorized_keys_path, existing_keys).unwrap();

    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![encrypted_key_path],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    let handler = PassphraseSshAuthHandler {
        passphrase: passphrase.to_string(),
    };
    match ssh.authenticate(handler).await {
        AuthResult::Authenticated(_) => {}
        AuthResult::Failed { error, .. } => {
            panic!("Should be authenticated with encrypted key + passphrase: {error}")
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn identities_only_should_skip_agent_and_use_file(sshd: Sshd) {
    // With identities_only=true, only the specified key file should be tried
    // (agent auth is skipped). This is already the pattern used by load_ssh_client,
    // but we make the behavior explicit here.
    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![sshd.tmp.child("id_ed25519").path().to_path_buf()],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    match ssh.authenticate(MockSshAuthHandler).await {
        AuthResult::Authenticated(_) => {}
        AuthResult::Failed { error, .. } => {
            panic!("Should authenticate via file-based key with identities_only=true: {error}")
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn authenticate_with_wrong_key_should_fail(sshd: Sshd) {
    // Generate a different key that is NOT in authorized_keys
    let key_dir = assert_fs::TempDir::new().unwrap();
    let wrong_key_path = key_dir.child("wrong_key").path().to_path_buf();
    assert!(
        SshKeygen::generate_ed25519(&wrong_key_path, "").expect("Failed to generate wrong key"),
        "ssh-keygen failed"
    );

    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![wrong_key_path],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    match ssh.authenticate(MockSshAuthHandler).await {
        AuthResult::Authenticated(_) => panic!("Authentication with wrong key should fail"),
        AuthResult::Failed { error, .. } => {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied,
                "Error should be PermissionDenied, got: {:?} - {}",
                error.kind(),
                error
            );
            let msg = error.to_string();
            assert!(
                msg.contains("Permission denied"),
                "Error message should contain 'Permission denied', got: '{msg}'"
            );
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn encrypted_key_with_wrong_passphrase_should_fail(sshd: Sshd) {
    let passphrase = "correct_passphrase";

    // Generate a passphrase-protected key
    let key_dir = assert_fs::TempDir::new().unwrap();
    let encrypted_key_path = key_dir.child("id_ed25519_enc").path().to_path_buf();
    assert!(
        SshKeygen::generate_ed25519(&encrypted_key_path, passphrase)
            .expect("Failed to generate encrypted key"),
        "ssh-keygen failed"
    );

    // Add the public key to authorized_keys
    let pub_key_contents =
        std::fs::read_to_string(encrypted_key_path.with_extension("pub")).unwrap();
    let authorized_keys_path = sshd.tmp.child("authorized_keys").path().to_path_buf();
    let mut existing_keys = std::fs::read_to_string(&authorized_keys_path).unwrap_or_default();
    existing_keys.push('\n');
    existing_keys.push_str(&pub_key_contents);
    std::fs::write(&authorized_keys_path, existing_keys).unwrap();

    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![encrypted_key_path],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    let handler = PassphraseSshAuthHandler {
        passphrase: "wrong_passphrase".to_string(),
    };

    // With wrong passphrase, the key decryption fails and auth falls through to error
    match ssh.authenticate(handler).await {
        AuthResult::Authenticated(_) => {
            panic!("Authentication with wrong passphrase should fail")
        }
        AuthResult::Failed { error, .. } => {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied,
                "Error should be PermissionDenied, got: {:?} - {}",
                error.kind(),
                error
            );
            let msg = error.to_string();
            assert!(
                msg.contains("Permission denied"),
                "Error message should contain 'Permission denied', got: '{msg}'"
            );
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn proxy_command_should_connect_via_tcp_to_stdio(sshd: Sshd) {
    let tcp_to_stdio = distant_test_harness::exe::build_tcp_to_stdio()
        .await
        .expect("failed to build tcp-to-stdio binary");

    let proxy_cmd = format!("{} 127.0.0.1:{}", tcp_to_stdio.display(), sshd.port);

    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![sshd.tmp.child("id_ed25519").path().to_path_buf()],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        proxy_command: Some(proxy_cmd),
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    match ssh.authenticate(MockSshAuthHandler).await {
        AuthResult::Authenticated(_) => {}
        AuthResult::Failed { error, .. } => {
            panic!("Should authenticate through ProxyCommand: {error}")
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn proxy_command_none_should_bypass_proxy(sshd: Sshd) {
    // "none" should disable ProxyCommand and connect directly
    let opts = SshOpts {
        port: Some(sshd.port),
        identity_files: vec![sshd.tmp.child("id_ed25519").path().to_path_buf()],
        identities_only: Some(true),
        user: Some(USERNAME.to_string()),
        user_known_hosts_files: vec![sshd.tmp.child("known_hosts").path().to_path_buf()],
        proxy_command: Some("none".to_string()),
        ..Default::default()
    };

    let ssh = SshSession::connect("127.0.0.1", opts).await.unwrap();
    match ssh.authenticate(MockSshAuthHandler).await {
        AuthResult::Authenticated(_) => {}
        AuthResult::Failed { error, .. } => {
            panic!("proxy_command='none' should fall back to direct connection: {error}")
        }
    }
}
