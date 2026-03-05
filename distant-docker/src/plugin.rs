//! Plugin implementation for the Docker backend.
//!
//! Provides [`DockerPlugin`] which implements the distant [`Plugin`] trait,
//! handling `"docker"` scheme destinations for both connecting to existing
//! containers and launching new ones.

use std::future::Future;
use std::io;
use std::pin::Pin;

use distant_core::Plugin;
use distant_core::auth::Authenticator;
use distant_core::net::client::UntypedClient;
use distant_core::net::common::{Destination, Map};
use log::*;

use crate::{Docker, DockerOpts, LaunchOpts};

/// Plugin for connecting to and launching Docker containers.
///
/// Handles the `"docker"` scheme. Connect attaches to an existing running container.
/// Launch creates a new container from an image and connects to it.
pub struct DockerPlugin;

impl Plugin for DockerPlugin {
    fn name(&self) -> &str {
        "docker"
    }

    fn connect<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
            debug!("Handling docker connect of {raw_destination} with options '{options}'");
            let container = raw_destination
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(raw_destination)
                .to_string();
            let docker_opts = parse_docker_opts(options);
            let docker = Docker::connect(&container, docker_opts).await?;
            Ok(docker.into_distant_client().await?.into_untyped_client())
        })
    }

    fn launch<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            debug!("Handling docker launch of {raw_destination} with options '{options}'");
            let image = raw_destination
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(raw_destination)
                .to_string();
            let docker_opts = parse_docker_opts(options);

            let auto_remove = options
                .get("auto_remove")
                .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");

            let launch_opts = LaunchOpts { image, auto_remove };

            let docker = Docker::launch(launch_opts, docker_opts).await?;
            let container = docker.container().to_string();

            // Return a destination pointing to the launched container
            Ok(Destination {
                scheme: Some("docker".to_string()),
                host: container.into(),
                port: None,
                username: None,
                password: None,
            })
        })
    }
}

/// Parse Docker-specific options from the options map.
fn parse_docker_opts(options: &Map) -> DockerOpts {
    DockerOpts {
        docker_host: options
            .get("docker_host")
            .or_else(|| options.get("docker.host"))
            .cloned(),
        user: options
            .get("user")
            .or_else(|| options.get("docker.user"))
            .cloned(),
        working_dir: options
            .get("working_dir")
            .or_else(|| options.get("docker.working_dir"))
            .cloned(),
        shell: options
            .get("shell")
            .or_else(|| options.get("docker.shell"))
            .cloned(),
    }
}
