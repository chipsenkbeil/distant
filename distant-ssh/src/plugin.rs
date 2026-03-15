//! Plugin implementation for the SSH backend.
//!
//! Provides [`SshPlugin`] which implements the distant [`Plugin`] trait,
//! handling `"ssh"` scheme destinations for both connecting to and launching
//! distant servers on remote hosts via SSH.

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;

use distant_core::Plugin;
use distant_core::auth::Authenticator;
use distant_core::auth::msg::*;
use distant_core::net::client::UntypedClient;
use distant_core::net::common::{Destination, Map};
use log::*;
use tokio::sync::Mutex;

use crate::{AuthResult, LaunchOpts, SshAuthEvent, SshAuthHandler, SshOpts, SshSession};

/// Plugin for launching and connecting via SSH.
///
/// Handles the `"ssh"` scheme. Launch uses SSH to start a distant server on the remote host.
/// Connect establishes a direct SSH connection and wraps it as a distant client.
pub struct SshPlugin;

impl Plugin for SshPlugin {
    fn name(&self) -> &str {
        "ssh"
    }

    fn connect<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
            let destination = distant_core::parse_destination(raw_destination)?;
            debug!("Handling connect of {destination} with options '{options}'");
            let ssh = load_ssh(&destination, options).await?;
            let handler = AuthClientSshAuthHandler::new(authenticator);
            match ssh.authenticate(handler).await {
                AuthResult::Authenticated(ssh) => {
                    Ok(ssh.into_distant_client().await?.into_untyped_client())
                }
                AuthResult::Failed { error, .. } => Err(error),
            }
        })
    }

    fn launch<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            let destination = distant_core::parse_destination(raw_destination)?;
            debug!("Handling launch of {destination} with options '{options}'");

            let ssh = load_ssh(&destination, options).await?;
            let handler = AuthClientSshAuthHandler::new(authenticator);
            let ssh = match ssh.authenticate(handler).await {
                AuthResult::Authenticated(ssh) => ssh,
                AuthResult::Failed { error, .. } => return Err(error),
            };

            let defaults = LaunchOpts::default();
            let opts = LaunchOpts {
                binary: options
                    .get("distant.bin")
                    .cloned()
                    .unwrap_or(defaults.binary),
                args: options
                    .get("distant.args")
                    .cloned()
                    .unwrap_or(defaults.args),
                timeout: match options.get("timeout") {
                    Some(s) => std::time::Duration::from_millis(
                        s.parse::<u64>().map_err(|_| invalid("timeout"))?,
                    ),
                    None => defaults.timeout,
                },
            };

            debug!("Launching via ssh: {opts:?}");
            ssh.launch(opts).await?.try_to_destination()
        })
    }
}

/// Adapter that bridges distant's [`Authenticator`] protocol with SSH authentication events.
struct AuthClientSshAuthHandler<'a>(Mutex<&'a mut dyn Authenticator>);

impl<'a> AuthClientSshAuthHandler<'a> {
    pub fn new(authenticator: &'a mut dyn Authenticator) -> Self {
        Self(Mutex::new(authenticator))
    }
}

impl<'a> SshAuthHandler for AuthClientSshAuthHandler<'a> {
    async fn on_authenticate(&self, event: SshAuthEvent) -> io::Result<Vec<String>> {
        use std::collections::HashMap;
        let mut options = HashMap::new();
        let mut questions = Vec::new();

        for prompt in event.prompts {
            let mut options = HashMap::new();
            options.insert("echo".to_string(), prompt.echo.to_string());
            questions.push(Question {
                label: "ssh-prompt".to_string(),
                text: prompt.prompt,
                options,
            });
        }

        options.insert("instructions".to_string(), event.instructions);
        options.insert("username".to_string(), event.username);

        Ok(self
            .0
            .lock()
            .await
            .challenge(Challenge { questions, options })
            .await?
            .answers
            .into_iter()
            .map(|s| s.into_exposed())
            .collect())
    }

    async fn on_verify_host<'b>(&'b self, host: &'b str) -> io::Result<bool> {
        Ok(self
            .0
            .lock()
            .await
            .verify(Verification {
                kind: VerificationKind::Host,
                text: host.to_string(),
            })
            .await?
            .valid)
    }

    async fn on_banner<'b>(&'b self, text: &'b str) {
        if let Err(x) = self
            .0
            .lock()
            .await
            .info(Info {
                text: text.to_string(),
            })
            .await
        {
            error!("ssh on_banner failed: {}", x);
        }
    }

    async fn on_error<'b>(&'b self, text: &'b str) {
        if let Err(x) = self
            .0
            .lock()
            .await
            .error(Error {
                kind: ErrorKind::Fatal,
                text: text.to_string(),
            })
            .await
        {
            error!("ssh on_error failed: {}", x);
        }
    }
}

/// Parse SSH connection options from a destination and options map.
fn parse_ssh_opts(destination: &Destination, options: &Map) -> io::Result<SshOpts> {
    Ok(SshOpts {
        identity_files: options
            .get("identity_files")
            .or_else(|| options.get("ssh.identity_files"))
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        identities_only: match options
            .get("identities_only")
            .or_else(|| options.get("ssh.identities_only"))
        {
            Some(s) => Some(s.parse().map_err(|_| invalid("identities_only"))?),
            None => None,
        },

        port: destination.port,

        proxy_command: options
            .get("proxy_command")
            .or_else(|| options.get("ssh.proxy_command"))
            .cloned(),

        user: destination.username.clone(),

        user_known_hosts_files: options
            .get("user_known_hosts_files")
            .or_else(|| options.get("ssh.user_known_hosts_files"))
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        verbose: match options
            .get("verbose")
            .or_else(|| options.get("ssh.verbose"))
            .or_else(|| options.get("client.verbose"))
        {
            Some(s) => s.parse().map_err(|_| invalid("verbose"))?,
            None => false,
        },

        other: {
            let mut other = std::collections::BTreeMap::new();
            if let Some(v) = options
                .get("strict_host_key_checking")
                .or_else(|| options.get("ssh.strict_host_key_checking"))
                .or_else(|| options.get("StrictHostKeyChecking"))
            {
                other.insert("stricthostkeychecking".to_string(), v.clone());
            }
            other
        },
    })
}

/// Load an SSH connection from a destination and options.
async fn load_ssh(destination: &Destination, options: &Map) -> io::Result<SshSession> {
    trace!("load_ssh({destination}, {options})");

    let host = destination.host.to_string();
    let opts = parse_ssh_opts(destination, options)?;

    debug!("Connecting to {host} via ssh with {opts:?}");
    SshSession::connect(host, opts).await
}

#[inline]
fn invalid(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid {label}"))
}
