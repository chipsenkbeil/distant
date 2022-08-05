use crate::cli::{fixtures::*, utils::ThreadedReader};
use assert_fs::prelude::*;
use rstest::*;
use std::{process::Command, thread, time::Duration};

fn wait_a_bit() {
    wait_millis(250);
}

fn wait_even_longer() {
    wait_millis(500);
}

fn wait_millis(millis: u64) {
    thread::sleep(Duration::from_millis(millis));
}

#[rstest]
fn should_support_watching_a_single_file(mut action_std_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    // distant action watch {path}
    let mut child = action_std_cmd
        .args(&["watch", file.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    // Wait for the process to be ready
    wait_a_bit();

    // Manipulate the file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the change is detected and reported
    wait_even_longer();

    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    let mut stdout_data = String::new();
    while let Some(line) = stdout.try_read_line_timeout(ThreadedReader::default_timeout()) {
        stdout_data.push_str(&line);
    }

    // Close out the process and collect the output
    child.kill().expect("Failed to terminate process");
    let output = child.wait_with_output().expect("Failed to wait for output");
    let stderr_data = String::from_utf8_lossy(&output.stderr).to_string();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Verify we get information printed out about the change
    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
    assert_eq!(stderr_data, "");
}

#[rstest]
fn should_support_watching_a_directory_recursively(mut action_std_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    // distant action watch {path}
    let mut child = action_std_cmd
        .args(&["watch", "--recursive", temp.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    // Wait for the process to be ready
    wait_a_bit();

    // Manipulate the file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the change is detected and reported
    wait_even_longer();

    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    let mut stdout_data = String::new();
    while let Some(line) = stdout.try_read_line_timeout(ThreadedReader::default_timeout()) {
        stdout_data.push_str(&line);
    }

    // Close out the process and collect the output
    child.kill().expect("Failed to terminate process");
    let output = child.wait_with_output().expect("Failed to wait for output");
    let stderr_data = String::from_utf8_lossy(&output.stderr).to_string();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Verify we get information printed out about the change
    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
    assert_eq!(stderr_data, "");
}

#[rstest]
fn yield_an_error_when_fails(mut action_std_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();
    let invalid_path = temp.to_path_buf().join("missing");

    // distant action watch {path}
    let child = action_std_cmd
        .args(&["watch", invalid_path.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    // Pause a bit to ensure that the process started and failed
    wait_a_bit();

    let output = child
        .wait_with_output()
        .expect("Failed to wait for child to complete");

    // Verify we get information printed out about the change
    assert!(!output.status.success(), "Child unexpectedly succeeded");
    assert!(output.stdout.is_empty(), "Unexpectedly got stdout");
    assert!(!output.stderr.is_empty(), "Missing stderr output");
}
