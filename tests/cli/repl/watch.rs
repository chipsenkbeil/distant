use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;
use std::time::Duration;

async fn wait_a_bit() {
    wait_millis(250).await;
}

async fn wait_even_longer() {
    wait_millis(500).await;
}

async fn wait_millis(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

#[rstest]
#[tokio::test]
async fn should_support_json_watching_single_file(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    // Watch single file for changes
    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "watch",
            "path": file.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        })
    );

    // Make a change to some file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_even_longer().await;

    // Get the response and verify the change
    // NOTE: Don't bother checking the kind as it can vary by platform
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "changed");
    assert_eq!(
        res["payload"]["paths"],
        json!([file.to_path_buf().canonicalize().unwrap()])
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_watching_directory_recursively(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    // Watch a directory recursively for changes
    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "watch",
            "path": temp.to_path_buf(),
            "recursive": true,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        })
    );

    // Make a change to some file
    file.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_even_longer().await;

    // Get the response and verify the change
    // NOTE: Don't bother checking the kind as it can vary by platform
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "changed");
    assert_eq!(
        res["payload"]["paths"],
        json!([file.to_path_buf().canonicalize().unwrap()])
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_reporting_changes_using_correct_request_id(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file1 = temp.child("file1");
    file1.touch().unwrap();

    let file2 = temp.child("file2");
    file2.touch().unwrap();

    // Watch file1 for changes
    let id_1 = rand::random::<u64>().to_string();
    let req = json!({
        "id": id_1,
        "payload": {
            "type": "watch",
            "path": file1.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id_1);
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        })
    );

    // Watch file2 for changes
    let id_2 = rand::random::<u64>().to_string();
    let req = json!({
        "id": id_2,
        "payload": {
            "type": "watch",
            "path": file2.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id_2);
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        })
    );

    // Make a change to file1
    file1.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_even_longer().await;

    // Get the response and verify the change
    // NOTE: Don't bother checking the kind as it can vary by platform
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id_1);
    assert_eq!(res["payload"]["type"], "changed");
    assert_eq!(
        res["payload"]["paths"],
        json!([file1.to_path_buf().canonicalize().unwrap()])
    );

    // Process any extra messages (we might get create, content, and more)
    loop {
        // Sleep a bit to give time to get all changes happening
        wait_a_bit().await;

        if json_repl
            .try_read_line_from_stdout()
            .expect("stdout closed unexpectedly")
            .is_none()
        {
            break;
        }
    }

    // Make a change to file2
    file2.write_str("some text").unwrap();

    // Pause a bit to ensure that the process detected the change and reported it
    wait_even_longer().await;

    // Get the response and verify the change
    // NOTE: Don't bother checking the kind as it can vary by platform
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id_2);
    assert_eq!(res["payload"]["type"], "changed");
    assert_eq!(
        res["payload"]["paths"],
        json!([file2.to_path_buf().canonicalize().unwrap()])
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();
    let path = temp.to_path_buf().join("missing");

    // Watch a missing path for changes
    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "watch",
            "path": path,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    // Pause a bit to ensure that the process started and processed our request
    wait_even_longer().await;

    // Ensure we got an acknowledgement of watching
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");
}
