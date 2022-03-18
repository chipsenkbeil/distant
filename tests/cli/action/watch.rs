use crate::cli::{fixtures::*, utils::random_tenant};
use assert_fs::prelude::*;
use distant_core::{
    data::{ChangeKind, ChangeKindSet, ErrorKind},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;
use std::{
    io,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn wait_a_bit() {
    wait_millis(50);
}

fn wait_millis(millis: u64) {
    thread::sleep(Duration::from_millis(millis));
}

struct ThreadedReader {
    handle: thread::JoinHandle<io::Result<()>>,
    rx: mpsc::Receiver<String>,
}

impl ThreadedReader {
    pub fn new<R>(reader: R) -> Self
    where
        R: Read + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line) {
                    Ok(0) => break Ok(()),
                    Ok(_) => {
                        // Consume the line and create an empty line to
                        // be filled in next time
                        let line2 = line;
                        line = String::new();

                        if let Err(line) = tx.send(line2) {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                format!(
                                    "Failed to pass along line because channel closed! Line: '{}'",
                                    line.0
                                ),
                            ));
                        }
                    }
                    Err(x) => return Err(x),
                }
            }
        });
        Self { handle, rx }
    }

    /// Tries to read the next line if available
    pub fn try_read_line(&mut self) -> Option<String> {
        self.rx.try_recv().ok()
    }

    /// Tries to read the next response if available
    ///
    /// Will panic if next line is not a valid response
    pub fn try_read_response(&mut self) -> Option<Response> {
        self.try_read_line().map(|line| {
            serde_json::from_str(&line)
                .unwrap_or_else(|_| panic!("Invalid response format for {}", line))
        })
    }

    /// Reads the next response, waiting for at minimum "timeout" before panicking
    pub fn read_response_timeout(&mut self, timeout: Duration) -> Response {
        let start_time = Instant::now();
        let mut checked_at_least_once = false;

        while !checked_at_least_once || start_time.elapsed() < timeout {
            if let Some(res) = self.try_read_response() {
                return res;
            }

            checked_at_least_once = true;
        }

        panic!("Reached timeout of {:?}", timeout);
    }

    /// Reads the next response, waiting for at minimum default timeout before panicking
    pub fn read_response_default_timeout(&mut self) -> Response {
        self.read_response_timeout(Self::default_timeout())
    }

    /// Creates a new duration representing a default timeout for the threaded reader
    pub fn default_timeout() -> Duration {
        Duration::from_millis(250)
    }

    /// Waits for reader to shut down, returning the result
    pub fn wait(self) -> io::Result<()> {
        match self.handle.join() {
            Ok(x) => x,
            Err(x) => std::panic::resume_unwind(x),
        }
    }
}

fn send_watch_request<W>(
    writer: &mut W,
    reader: &mut ThreadedReader,
    path: impl Into<PathBuf>,
    recursive: bool,
    only: impl Into<ChangeKindSet>,
) -> Response
where
    W: Write,
{
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Watch {
            path: path.into(),
            recursive,
            only: only.into(),
        }],
    };

    // Send our request to the process
    let msg = format!("{}\n", serde_json::to_string(&req).unwrap());
    writer
        .write_all(msg.as_bytes())
        .expect("Failed to write to process");

    // Pause a bit to ensure that the process started and processed our request
    wait_a_bit();

    // Ensure we got an acknowledgement of watching
    let res = reader.read_response_default_timeout();
    assert_eq!(res.payload[0], ResponseData::Ok);
    res
}

#[rstest]
fn should_support_watching_a_single_file(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    // distant action watch {path}
    let mut child = action_std_cmd
        .args(&["watch", "--only", "modify_data", file.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    // Wait for the process to be ready
    wait_a_bit();

    // Manipulate the file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the change is detected and reported
    wait_a_bit();

    // Close out the process and collect the output
    let _ = child.kill().expect("Failed to terminate process");
    let output = child.wait_with_output().expect("Failed to wait for output");
    let stdout_data = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_data = String::from_utf8_lossy(&output.stderr).to_string();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Verify we get information printed out about the change
    assert_eq!(
        stdout_data,
        format!("Following paths were modified:\n* {}\n", path)
    );
    assert_eq!(stderr_data, "");
}

#[rstest]
fn should_support_watching_a_directory_recursively(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    // distant action watch {path}
    let mut child = action_std_cmd
        .args(&[
            "watch",
            "--recursive",
            "--only",
            "modify_data",
            temp.to_str().unwrap(),
        ])
        .spawn()
        .expect("Failed to execute");

    // Wait for the process to be ready
    wait_a_bit();

    // Manipulate the file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the change is detected and reported
    wait_a_bit();

    // Close out the process and collect the output
    let _ = child.kill().expect("Failed to terminate process");
    let output = child.wait_with_output().expect("Failed to wait for output");
    let stdout_data = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_data = String::from_utf8_lossy(&output.stderr).to_string();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Verify we get information printed out about the change
    assert_eq!(
        stdout_data,
        format!("Following paths were modified:\n* {}\n", path)
    );
    assert_eq!(stderr_data, "");
}

#[rstest]
fn yield_an_error_when_fails(mut action_std_cmd: Command) {
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

#[rstest]
fn should_support_json_watching_single_file(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    // distant action --format json --interactive
    let mut cmd = action_std_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .expect("Failed to execute");
    let mut stdin = cmd.stdin.take().unwrap();
    let mut stdout = ThreadedReader::new(cmd.stdout.take().unwrap());

    let _ = send_watch_request(
        &mut stdin,
        &mut stdout,
        file.to_path_buf(),
        false,
        ChangeKind::ModifyData,
    );

    // Make a change to some file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_a_bit();

    // Get the response and verify the change
    let res = stdout.read_response_default_timeout();
    match &res.payload[0] {
        ResponseData::Changed(change) => {
            assert_eq!(change.kind, ChangeKind::ModifyData);
            assert_eq!(&change.paths, &[file.to_path_buf().canonicalize().unwrap()]);
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
fn should_support_json_watching_directory_recursively(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    // distant action --format json --interactive
    let mut cmd = action_std_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .expect("Failed to execute");
    let mut stdin = cmd.stdin.take().unwrap();
    let mut stdout = ThreadedReader::new(cmd.stdout.take().unwrap());

    let _ = send_watch_request(
        &mut stdin,
        &mut stdout,
        temp.to_path_buf(),
        true,
        ChangeKind::ModifyData,
    );

    // Make a change to some file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_a_bit();

    // Get the response and verify the change
    let res = stdout.read_response_default_timeout();
    match &res.payload[0] {
        ResponseData::Changed(change) => {
            assert_eq!(change.kind, ChangeKind::ModifyData);
            assert_eq!(&change.paths, &[file.to_path_buf().canonicalize().unwrap()]);
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
fn should_support_json_reporting_changes_using_correct_request_id(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file1 = temp.child("file1");
    file1.touch().unwrap();

    let file2 = temp.child("file2");
    file2.touch().unwrap();

    // distant action --format json --interactive
    let mut cmd = action_std_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .expect("Failed to execute");
    let mut stdin = cmd.stdin.take().unwrap();
    let mut stdout = ThreadedReader::new(cmd.stdout.take().unwrap());

    // Create a request to watch file1
    let file1_res = send_watch_request(
        &mut stdin,
        &mut stdout,
        file1.to_path_buf(),
        true,
        ChangeKind::ModifyData,
    );

    // Create a request to watch file2
    let file2_res = send_watch_request(
        &mut stdin,
        &mut stdout,
        file2.to_path_buf(),
        true,
        ChangeKind::ModifyData,
    );

    assert_ne!(
        file1_res.origin_id, file2_res.origin_id,
        "Two separate watch responses have same origin id"
    );

    // Make a change to file1
    file1.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_a_bit();

    // Get the response and verify the change
    let file1_change_res = stdout.read_response_default_timeout();
    match &file1_change_res.payload[0] {
        ResponseData::Changed(change) => {
            assert_eq!(change.kind, ChangeKind::ModifyData);
            assert_eq!(
                &change.paths,
                &[file1.to_path_buf().canonicalize().unwrap()]
            );
        }
        x => panic!("Unexpected response: {:?}", x),
    }

    // Make a change to file2
    file2.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_a_bit();

    // Get the response and verify the change
    let file2_change_res = stdout.read_response_default_timeout();
    match &file2_change_res.payload[0] {
        ResponseData::Changed(change) => {
            assert_eq!(change.kind, ChangeKind::ModifyData);
            assert_eq!(
                &change.paths,
                &[file2.to_path_buf().canonicalize().unwrap()]
            );
        }
        x => panic!("Unexpected response: {:?}", x),
    }

    // Verify that the response origin ids match and are separate
    assert_eq!(
        file1_res.origin_id, file1_change_res.origin_id,
        "File 1 watch origin and change origin are different"
    );
    assert_eq!(
        file2_res.origin_id, file2_change_res.origin_id,
        "File 1 watch origin and change origin are different"
    );
    assert_ne!(
        file1_change_res.origin_id, file2_change_res.origin_id,
        "Two separate watch change responses have same origin id"
    );
}

#[rstest]
fn should_support_json_output_for_error(mut action_std_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let path = temp.to_path_buf().join("missing");

    // distant action --format json --interactive
    let mut cmd = action_std_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .expect("Failed to execute");
    let mut stdin = cmd.stdin.take().unwrap();
    let mut stdout = ThreadedReader::new(cmd.stdout.take().unwrap());

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Watch {
            path,
            recursive: false,
            only: Default::default(),
        }],
    };

    // Send our request to the process
    let msg = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin
        .write_all(msg.as_bytes())
        .expect("Failed to write to process");

    // Pause a bit to ensure that the process started and processed our request
    wait_a_bit();

    // Ensure we got an acknowledgement of watching
    let res = stdout.read_response_default_timeout();
    match &res.payload[0] {
        ResponseData::Error(x) => {
            assert_eq!(x.kind, ErrorKind::NotFound);
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}
