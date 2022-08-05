use crate::cli::fixtures::*;
use assert_cmd::Command;
use rstest::*;
use std::env;

#[rstest]
fn should_output_system_info(mut action_cmd: CtxCommand<Command>) {
    // distant action system-info
    action_cmd
        .arg("system-info")
        .assert()
        .success()
        .stdout(format!(
            concat!(
                "Family: {:?}\n",
                "Operating System: {:?}\n",
                "Arch: {:?}\n",
                "Cwd: {:?}\n",
                "Path Sep: {:?}\n",
            ),
            env::consts::FAMILY.to_string(),
            env::consts::OS.to_string(),
            env::consts::ARCH.to_string(),
            env::current_dir().unwrap_or_default(),
            std::path::MAIN_SEPARATOR,
        ))
        .stderr("");
}
