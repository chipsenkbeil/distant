/// Path to distant binary
#[inline]
fn bin_path() -> PathBuf {
    assert_cmd::cargo::cargo_bin(env!("CARGO_PKG_NAME"))
}

fn start_manager() {
    // Start the manager
    let mut manager_cmd = StdCommand::new(bin_path());
    manager_cmd
        .arg("manager")
        .arg("listen")
        .arg("--log-file")
        .arg(random_log_file("manager"))
        .arg("--log-level")
        .arg("trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
}
