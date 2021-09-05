use crate::cli::{
    fixtures::*,
    utils::{random_tenant, FAILURE_LINE},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{DirEntry, Error, ErrorKind, FileType},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;
use std::path::PathBuf;

/// Creates a directory in the form
///
/// $TEMP/
/// $TEMP/dir1/
/// $TEMP/dir1/dira/
/// $TEMP/dir1/dirb/
/// $TEMP/dir1/dirb/file1
/// $TEMP/dir1/file1
/// $TEMP/dir1/file2
/// $TEMP/dir2/
/// $TEMP/dir2/dira/
/// $TEMP/dir2/dirb/
/// $TEMP/dir2/dirb/file1
/// $TEMP/dir2/file1
/// $TEMP/dir2/file2
/// $TEMP/file1
/// $TEMP/file2
fn make_directory() -> assert_fs::TempDir {
    let temp = assert_fs::TempDir::new().unwrap();

    // $TEMP/file1
    // $TEMP/file2
    temp.child("file1").touch().unwrap();
    temp.child("file2").touch().unwrap();

    // $TEMP/dir1/
    // $TEMP/dir1/file1
    // $TEMP/dir1/file2
    let dir1 = temp.child("dir1");
    dir1.create_dir_all().unwrap();
    dir1.child("file1").touch().unwrap();
    dir1.child("file2").touch().unwrap();

    // $TEMP/dir1/dira/
    let dir1_dira = dir1.child("dira");
    dir1_dira.create_dir_all().unwrap();

    // $TEMP/dir1/dirb/
    // $TEMP/dir1/dirb/file1
    let dir1_dirb = dir1.child("dirb");
    dir1_dirb.create_dir_all().unwrap();
    dir1_dirb.child("file1").touch().unwrap();

    // $TEMP/dir2/
    // $TEMP/dir2/file1
    // $TEMP/dir2/file2
    let dir2 = temp.child("dir2");
    dir2.create_dir_all().unwrap();
    dir2.child("file1").touch().unwrap();
    dir2.child("file2").touch().unwrap();

    // $TEMP/dir2/dira/
    let dir2_dira = dir2.child("dira");
    dir2_dira.create_dir_all().unwrap();

    // $TEMP/dir2/dirb/
    // $TEMP/dir2/dirb/file1
    let dir2_dirb = dir2.child("dirb");
    dir2_dirb.create_dir_all().unwrap();
    dir2_dirb.child("file1").touch().unwrap();

    temp
}

#[rstest]
fn should_print_immediate_files_and_directories_by_default(mut action_cmd: Command) {
    let temp = make_directory();

    // distant action dir-read {path}
    action_cmd
        .args(&["dir-read", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(concat!("dir1/\n", "dir2/\n", "file1\n", "file2\n"))
        .stderr("");
}

#[rstest]
fn should_use_absolute_paths_if_specified(mut action_cmd: Command) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so the absolute path
    //       provided is our canonicalized root path prepended
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    // distant action dir-read --absolute {path}
    action_cmd
        .args(&["dir-read", "--absolute", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            vec![
                format!("{}/{}", root_path.to_str().unwrap(), "dir1/"),
                format!("{}/{}", root_path.to_str().unwrap(), "dir2/"),
                format!("{}/{}", root_path.to_str().unwrap(), "file1"),
                format!("{}/{}", root_path.to_str().unwrap(), "file2"),
            ]
            .join("\n")
        ))
        .stderr("");
}

#[rstest]
fn should_print_all_files_and_directories_if_depth_is_0(mut action_cmd: Command) {
    let temp = make_directory();

    // distant action dir-read --depth 0 {path}
    action_cmd
        .args(&["dir-read", "--depth", "0", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(concat!(
            "dir1/\n",
            "dir1/dira/\n",
            "dir1/dirb/\n",
            "dir1/dirb/file1\n",
            "dir1/file1\n",
            "dir1/file2\n",
            "dir2/\n",
            "dir2/dira/\n",
            "dir2/dirb/\n",
            "dir2/dirb/file1\n",
            "dir2/file1\n",
            "dir2/file2\n",
            "file1\n",
            "file2\n",
        ))
        .stderr("");
}

#[rstest]
fn should_include_root_directory_if_specified(mut action_cmd: Command) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so yielded entry
    //       is the canonicalized version
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    // distant action dir-read --include-root {path}
    action_cmd
        .args(&["dir-read", "--include-root", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(format!(
            "{}/\n{}",
            root_path.to_str().unwrap(),
            concat!("dir1/\n", "dir2/\n", "file1\n", "file2\n")
        ))
        .stderr("");
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = make_directory();
    let dir = temp.child("missing-dir");

    // distant action dir-read {path}
    action_cmd
        .args(&["dir-read", dir.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}

#[rstest]
fn should_support_json_output(mut action_cmd: Command) {
    let temp = make_directory();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::DirRead {
            path: temp.to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
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
    assert_eq!(
        res.payload[0],
        ResponseData::DirEntries {
            entries: vec![
                DirEntry {
                    path: PathBuf::from("dir1"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("dir2"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("file1"),
                    file_type: FileType::File,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("file2"),
                    file_type: FileType::File,
                    depth: 1
                },
            ],
            errors: Vec::new(),
        }
    );
}

#[rstest]
fn should_support_json_returning_absolute_paths_if_specified(mut action_cmd: Command) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so the absolute path
    //       provided is our canonicalized root path prepended
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::DirRead {
            path: temp.to_path_buf(),
            depth: 1,
            absolute: true,
            canonicalize: false,
            include_root: false,
        }],
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
    assert_eq!(
        res.payload[0],
        ResponseData::DirEntries {
            entries: vec![
                DirEntry {
                    path: root_path.join("dir1"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: root_path.join("dir2"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: root_path.join("file1"),
                    file_type: FileType::File,
                    depth: 1
                },
                DirEntry {
                    path: root_path.join("file2"),
                    file_type: FileType::File,
                    depth: 1
                },
            ],
            errors: Vec::new(),
        }
    );
}

#[rstest]
fn should_support_json_returning_all_files_and_directories_if_depth_is_0(mut action_cmd: Command) {
    let temp = make_directory();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::DirRead {
            path: temp.to_path_buf(),
            depth: 0,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
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
    assert_eq!(
        res.payload[0],
        ResponseData::DirEntries {
            /* "dir1/\n",
            "dir1/dira/\n",
            "dir1/dirb/\n",
            "dir1/dirb/file1\n",
            "dir1/file1\n",
            "dir1/file2\n",
            "dir2/\n",
            "dir2/dira/\n",
            "dir2/dirb/\n",
            "dir2/dirb/file1\n",
            "dir2/file1\n",
            "dir2/file2\n",
            "file1\n",
            "file2\n", */
            entries: vec![
                DirEntry {
                    path: PathBuf::from("dir1"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("dir1").join("dira"),
                    file_type: FileType::Dir,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir1").join("dirb"),
                    file_type: FileType::Dir,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir1").join("dirb").join("file1"),
                    file_type: FileType::File,
                    depth: 3
                },
                DirEntry {
                    path: PathBuf::from("dir1").join("file1"),
                    file_type: FileType::File,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir1").join("file2"),
                    file_type: FileType::File,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir2"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("dir2").join("dira"),
                    file_type: FileType::Dir,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir2").join("dirb"),
                    file_type: FileType::Dir,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir2").join("dirb").join("file1"),
                    file_type: FileType::File,
                    depth: 3
                },
                DirEntry {
                    path: PathBuf::from("dir2").join("file1"),
                    file_type: FileType::File,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("dir2").join("file2"),
                    file_type: FileType::File,
                    depth: 2
                },
                DirEntry {
                    path: PathBuf::from("file1"),
                    file_type: FileType::File,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("file2"),
                    file_type: FileType::File,
                    depth: 1
                },
            ],
            errors: Vec::new(),
        }
    );
}

#[rstest]
fn should_support_json_including_root_directory_if_specified(mut action_cmd: Command) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so yielded entry
    //       is the canonicalized version
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::DirRead {
            path: temp.to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: false,
            include_root: true,
        }],
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
    assert_eq!(
        res.payload[0],
        ResponseData::DirEntries {
            entries: vec![
                DirEntry {
                    path: root_path,
                    file_type: FileType::Dir,
                    depth: 0
                },
                DirEntry {
                    path: PathBuf::from("dir1"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("dir2"),
                    file_type: FileType::Dir,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("file1"),
                    file_type: FileType::File,
                    depth: 1
                },
                DirEntry {
                    path: PathBuf::from("file2"),
                    file_type: FileType::File,
                    depth: 1
                },
            ],
            errors: Vec::new(),
        }
    );
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = make_directory();
    let dir = temp.child("missing-dir");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::DirRead {
            path: dir.to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
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
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Error(Error {
                kind: ErrorKind::NotFound,
                ..
            })
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}
