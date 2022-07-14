use crate::cli::{fixtures::*, utils::random_tenant};
use assert_cmd::Command;
use distant_core::{data::SystemInfo, Request, RequestData, Response, ResponseData};
use rstest::*;
use std::env;

#[rstest]
fn should_output_system_info(mut action_cmd: Command) {
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

#[rstest]
fn should_support_json_system_info(mut action_cmd: Command) {
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::SystemInfo {}],
    };

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", serde_json::to_string(&req).unwrap()))
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    match &res.payload[0] {
        ResponseData::SystemInfo(info) => {
            assert_eq!(
                info,
                &SystemInfo {
                    family: env::consts::FAMILY.to_string(),
                    os: env::consts::OS.to_string(),
                    arch: env::consts::ARCH.to_string(),
                    current_dir: env::current_dir().unwrap_or_default(),
                    main_separator: std::path::MAIN_SEPARATOR,
                }
            );
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}
