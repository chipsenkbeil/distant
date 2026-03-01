//! Tests for Docker connection, family detection, and container lifecycle.

use distant_docker::{Docker, DockerFamily, DockerOpts, LaunchOpts};
use distant_test_harness::docker::{DockerContainer, docker_container};
use distant_test_harness::skip_if_no_docker;
use rstest::*;
use test_log::test;

#[rstest]
#[test(tokio::test)]
async fn connect_should_succeed_for_running_container(
    #[future] docker_container: Option<DockerContainer>,
) {
    let container = skip_if_no_docker!(docker_container.await);
    let result = Docker::connect(&container.name, DockerOpts::default()).await;
    assert!(result.is_ok(), "Failed to connect: {:?}", result.err());
}

#[rstest]
#[test(tokio::test)]
async fn connect_should_fail_for_nonexistent_container() {
    if !distant_test_harness::docker::docker_available().await {
        return;
    }

    let result = Docker::connect("distant-nonexistent-container-xyz", DockerOpts::default()).await;
    assert!(result.is_err());
}

#[rstest]
#[test(tokio::test)]
async fn detect_family_should_return_unix_on_linux_container(
    #[future] docker_container: Option<DockerContainer>,
) {
    let container = skip_if_no_docker!(docker_container.await);
    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .expect("Failed to connect");

    if cfg!(windows) {
        assert_eq!(docker.family(), DockerFamily::Windows);
    } else {
        assert_eq!(docker.family(), DockerFamily::Unix);
    }
}

#[rstest]
#[test(tokio::test)]
async fn container_name_should_match(#[future] docker_container: Option<DockerContainer>) {
    let container = skip_if_no_docker!(docker_container.await);
    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .expect("Failed to connect");
    assert_eq!(docker.container(), container.name);
}

#[rstest]
#[test(tokio::test)]
async fn launch_should_create_and_connect_to_new_container() {
    if !distant_test_harness::docker::docker_available().await {
        return;
    }

    let image = if cfg!(windows) {
        "mcr.microsoft.com/windows/nanoserver:ltsc2025"
    } else {
        "ubuntu:22.04"
    };

    let launch_opts = LaunchOpts {
        image: image.to_string(),
        auto_remove: false,
    };

    let docker = Docker::launch(launch_opts, DockerOpts::default())
        .await
        .expect("Failed to launch container");

    let container_name = docker.container().to_string();
    assert!(container_name.starts_with("distant-"));

    // Clean up
    let client = Docker::default_bollard_client().unwrap();
    let _ = Docker::stop_and_remove(&client, &container_name).await;
}

#[rstest]
#[test(tokio::test)]
async fn into_distant_client_should_succeed(#[future] docker_container: Option<DockerContainer>) {
    let container = skip_if_no_docker!(docker_container.await);
    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .expect("Failed to connect");
    let result = docker.into_distant_client().await;
    assert!(
        result.is_ok(),
        "Failed to create client: {:?}",
        result.err()
    );
}
