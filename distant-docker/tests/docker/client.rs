//! Integration tests for distant Client operations through the Docker backend.

use std::path::PathBuf;

use distant_core::protocol::{
    FileType, SearchQueryCondition, SearchQueryMatch, SearchQueryOptions,
};
use distant_core::{ChannelExt, Client};
use distant_test_harness::docker::{Ctx, client, client_with_tunnel_tools};
use distant_test_harness::skip_if_no_docker;
use rstest::*;
use test_log::test;

/// Returns the temp directory path for the container.
fn test_temp_dir() -> PathBuf {
    PathBuf::from("/tmp")
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn write_file_and_read_file_should_roundtrip(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-roundtrip.txt");
    let data = b"hello from distant!";

    client
        .write_file(path.clone(), data.to_vec(), Default::default())
        .await
        .unwrap();
    let result = client
        .read_file(path.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(result, data);

    let _ = client.remove(path, false).await;
}

#[rstest]
#[test(tokio::test)]
async fn read_file_should_fail_if_missing(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-nonexistent-file.txt");
    let err = client
        .read_file(path, Default::default())
        .await
        .expect_err("Expected error reading nonexistent file");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn write_file_text_and_read_file_text_should_roundtrip(
    #[future] client: Option<Ctx<Client>>,
) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-text-roundtrip.txt");
    let text = "hello text from distant!";

    client
        .write_file_text(path.clone(), text.to_string())
        .await
        .unwrap();
    let result = client.read_file_text(path.clone()).await.unwrap();
    assert_eq!(result, text);

    let _ = client.remove(path, false).await;
}

#[rstest]
#[test(tokio::test)]
async fn append_file_should_append_data(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-append.txt");

    client
        .write_file(path.clone(), b"hello".to_vec(), Default::default())
        .await
        .unwrap();
    client
        .append_file(path.clone(), b" world".to_vec())
        .await
        .unwrap();

    let result = client
        .read_file(path.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(result, b"hello world");

    let _ = client.remove(path, false).await;
}

// ---------------------------------------------------------------------------
// Directory operations
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn create_dir_and_exists_should_work(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-dir");

    // Clean up any previous test run
    let _ = client.remove(path.clone(), true).await;

    client.create_dir(path.clone(), false).await.unwrap();
    let exists = client.exists(path.clone()).await.unwrap();
    assert!(exists);

    let _ = client.remove(path, true).await;
}

#[rstest]
#[test(tokio::test)]
async fn create_dir_all_should_create_nested_dirs(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let base = test_temp_dir().join("distant-test-nested");
    let path = base.join("a").join("b").join("c");

    let _ = client.remove(base.clone(), true).await;

    client.create_dir(path.clone(), true).await.unwrap();
    let exists = client.exists(path).await.unwrap();
    assert!(exists);

    let _ = client.remove(base, true).await;
}

#[rstest]
#[test(tokio::test)]
async fn read_dir_should_list_entries(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let dir = test_temp_dir().join("distant-test-readdir");

    let _ = client.remove(dir.clone(), true).await;
    client.create_dir(dir.clone(), false).await.unwrap();
    client
        .write_file(dir.join("file1.txt"), b"a".to_vec(), Default::default())
        .await
        .unwrap();
    client
        .write_file(dir.join("file2.txt"), b"b".to_vec(), Default::default())
        .await
        .unwrap();

    let (entries, _errors) = client
        .read_dir(dir.clone(), 0, false, false, false)
        .await
        .unwrap();

    let names: Vec<String> = entries
        .iter()
        .filter_map(|e| e.path.as_str().rsplit('/').next().map(|s| s.to_string()))
        .collect();

    assert!(names.contains(&"file1.txt".to_string()));
    assert!(names.contains(&"file2.txt".to_string()));

    let _ = client.remove(dir, true).await;
}

// ---------------------------------------------------------------------------
// Copy and Rename
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn copy_should_duplicate_file(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let src = test_temp_dir().join("distant-test-copy-src.txt");
    let dst = test_temp_dir().join("distant-test-copy-dst.txt");

    let _ = client.remove(dst.clone(), false).await;

    client
        .write_file(src.clone(), b"copy me".to_vec(), Default::default())
        .await
        .unwrap();
    client.copy(src.clone(), dst.clone()).await.unwrap();

    let result = client
        .read_file(dst.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(result, b"copy me");

    let _ = client.remove(src, false).await;
    let _ = client.remove(dst, false).await;
}

#[rstest]
#[test(tokio::test)]
async fn rename_should_move_file(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let src = test_temp_dir().join("distant-test-rename-src.txt");
    let dst = test_temp_dir().join("distant-test-rename-dst.txt");

    let _ = client.remove(dst.clone(), false).await;

    client
        .write_file(src.clone(), b"rename me".to_vec(), Default::default())
        .await
        .unwrap();
    client.rename(src.clone(), dst.clone()).await.unwrap();

    let exists_src = client.exists(src).await.unwrap();
    assert!(!exists_src, "Source should not exist after rename");

    let result = client
        .read_file(dst.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(result, b"rename me");

    let _ = client.remove(dst, false).await;
}

// ---------------------------------------------------------------------------
// Metadata and Exists
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn exists_should_return_false_for_missing_path(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let result = client
        .exists(test_temp_dir().join("distant-test-no-such-file"))
        .await
        .unwrap();
    assert!(!result);
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_return_file_info(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-metadata.txt");

    client
        .write_file(path.clone(), b"metadata test".to_vec(), Default::default())
        .await
        .unwrap();

    let metadata = client.metadata(path.clone(), false, false).await.unwrap();
    assert_eq!(metadata.file_type, FileType::File);
    assert_eq!(
        metadata.len, 13,
        "Expected file length 13 for 'metadata test', got: {}",
        metadata.len,
    );

    let _ = client.remove(path, false).await;
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_identify_directory(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-dir-meta");

    let _ = client.remove(path.clone(), true).await;
    client.create_dir(path.clone(), false).await.unwrap();

    let metadata = client.metadata(path.clone(), false, false).await.unwrap();
    assert_eq!(metadata.file_type, FileType::Dir);

    let _ = client.remove(path, true).await;
}

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn remove_should_delete_file(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-remove.txt");

    client
        .write_file(path.clone(), b"remove me".to_vec(), Default::default())
        .await
        .unwrap();
    client.remove(path.clone(), false).await.unwrap();

    let exists = client.exists(path).await.unwrap();
    assert!(!exists);
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_delete_directory_recursively(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let dir = test_temp_dir().join("distant-test-rmdir");

    let _ = client.remove(dir.clone(), true).await;
    let sub = dir.join("sub");
    client.create_dir(sub.clone(), true).await.unwrap();
    client
        .write_file(sub.join("file.txt"), b"nested".to_vec(), Default::default())
        .await
        .unwrap();

    client.remove(dir.clone(), true).await.unwrap();

    let exists = client.exists(dir).await.unwrap();
    assert!(!exists);
}

// ---------------------------------------------------------------------------
// Process spawn
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_execute_command(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);

    let proc = client
        .spawn("echo hello".to_string(), Default::default(), None, None)
        .await
        .unwrap();

    let output = proc.output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello"),
        "Expected process stdout to contain 'hello', got: {stdout}",
    );
}

// ---------------------------------------------------------------------------
// System info
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn system_info_should_return_valid_data(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let info = client.system_info().await.unwrap();

    assert_eq!(
        info.family, "unix",
        "Expected unix family, got: {}",
        info.family
    );
    assert_eq!(info.os, "linux", "Expected linux os, got: {}", info.os);
    assert!(
        ["x86_64", "aarch64"].contains(&info.arch.as_str()),
        "Unexpected arch: {}",
        info.arch,
    );
    assert!(!info.username.is_empty(), "Expected non-empty username");
    assert!(
        info.shell.contains("sh"),
        "Expected shell path containing 'sh', got: {}",
        info.shell,
    );
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn version_should_include_capabilities(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let version = client.version().await.unwrap();

    let expected_version: distant_core::protocol::semver::Version =
        env!("CARGO_PKG_VERSION").parse().unwrap();
    assert_eq!(
        version.server_version.major, expected_version.major,
        "Server major version mismatch",
    );
    assert_eq!(
        version.server_version.minor, expected_version.minor,
        "Server minor version mismatch",
    );
    assert_eq!(
        version.protocol_version,
        distant_core::protocol::PROTOCOL_VERSION,
        "Protocol version mismatch",
    );
    // Docker backend always supports these capabilities
    for expected_cap in ["exec", "fs_io", "sys_info", "fs_perm"] {
        assert!(
            version.capabilities.contains(&expected_cap.to_string()),
            "Missing expected capability '{expected_cap}', got: {:?}",
            version.capabilities,
        );
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn search_contents_should_find_pattern_in_file(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let dir = test_temp_dir().join("distant-test-search-contents");
    let file = dir.join("haystack.txt");

    let _ = client.remove(dir.clone(), true).await;
    client.create_dir(dir.clone(), true).await.unwrap();
    client
        .write_file_text(
            file.clone(),
            "line one\nthe needle is here\nline three\n".to_string(),
        )
        .await
        .unwrap();

    let query = distant_core::protocol::SearchQuery::contents(
        SearchQueryCondition::Contains {
            value: "needle".to_string(),
        },
        [dir.clone()],
        SearchQueryOptions::default(),
    );

    let mut searcher = client.search(query).await.unwrap();
    let mut found = Vec::new();
    while let Some(m) = searcher.next().await {
        found.push(m);
    }

    assert!(
        !found.is_empty(),
        "Expected at least one content search match"
    );
    let file_str = file.to_string_lossy().to_string();
    assert!(
        found
            .iter()
            .any(|m| matches!(m, SearchQueryMatch::Contents(c) if c.path.as_str() == file_str))
    );

    let _ = client.remove(dir, true).await;
}

#[rstest]
#[test(tokio::test)]
async fn search_path_should_find_file_by_name(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let dir = test_temp_dir().join("distant-test-search-path");
    let file = dir.join("unique-needle-file.txt");

    let _ = client.remove(dir.clone(), true).await;
    client.create_dir(dir.clone(), true).await.unwrap();
    client
        .write_file(file, b"content".to_vec(), Default::default())
        .await
        .unwrap();

    let query = distant_core::protocol::SearchQuery::path(
        SearchQueryCondition::Contains {
            value: "unique-needle-file".to_string(),
        },
        [dir.clone()],
        SearchQueryOptions::default(),
    );

    let mut searcher = client.search(query).await.unwrap();
    let mut found = Vec::new();
    while let Some(m) = searcher.next().await {
        found.push(m);
    }

    assert!(!found.is_empty(), "Expected at least one path search match");
    assert!(found.iter().any(
        |m| matches!(m, SearchQueryMatch::Path(p) if p.path.as_str().contains("unique-needle-file"))
    ));

    let _ = client.remove(dir, true).await;
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn append_file_should_fail_for_invalid_path(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = PathBuf::from("/nonexistent-dir/distant-test-append.txt");
    let err = client
        .append_file(path, b"data".to_vec())
        .await
        .expect_err("Expected error appending to invalid path");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected path-not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn copy_should_fail_for_missing_source(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let src = test_temp_dir().join("distant-test-copy-missing-src.txt");
    let dst = test_temp_dir().join("distant-test-copy-missing-dst.txt");
    let err = client
        .copy(src, dst)
        .await
        .expect_err("Expected error copying from missing source");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected source-not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn rename_should_fail_for_missing_source(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let src = test_temp_dir().join("distant-test-rename-missing-src.txt");
    let dst = test_temp_dir().join("distant-test-rename-missing-dst.txt");
    let err = client
        .rename(src, dst)
        .await
        .expect_err("Expected error renaming missing source");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected source-not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn create_dir_should_fail_when_parent_missing(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = PathBuf::from("/nonexistent-parent/distant-test-dir");
    let err = client
        .create_dir(path, false)
        .await
        .expect_err("Expected error creating dir with missing parent");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_fail_for_missing_path(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-no-such-file-metadata");
    let err = client
        .metadata(path, false, false)
        .await
        .expect_err("Expected error getting metadata for missing path");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("not found")
            || err_msg.contains("no such file")
            || err_msg.contains("404"),
        "Expected not-found error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_open_should_fail_without_relay_tools(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let result = client.tunnel_open("127.0.0.1", 12345).await;
    let err = result.expect_err("Expected error without tunnel tools");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("unsupported") || err_msg.contains("no tunnel relay tools"),
        "Expected unsupported error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_open_should_send_and_receive_data(
    #[future] client_with_tunnel_tools: Option<Ctx<Client>>,
) {
    let mut client = skip_if_no_docker!(client_with_tunnel_tools.await);

    // Start a listener inside the container that sends data to whoever connects
    let _proc = client
        .spawn(
            "sh -c 'echo HELLO | nc -l -p 17777'".to_string(),
            Default::default(),
            None,
            None,
        )
        .await
        .unwrap();

    // Give the listener time to start accepting connections
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Open tunnel to the in-container listener
    let mut tunnel = client.tunnel_open("127.0.0.1", 17777).await.unwrap();
    let mut reader = tunnel.reader.take().unwrap();

    // Read the data sent by the nc listener
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    let mut accumulated = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, reader.read()).await {
            Ok(Ok(data)) => {
                accumulated.extend_from_slice(&data);
                let s = String::from_utf8_lossy(&accumulated);
                if s.contains("HELLO") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => panic!("Timed out waiting for tunnel data"),
        }
    }

    let received = String::from_utf8_lossy(&accumulated);
    assert!(
        received.contains("HELLO"),
        "Expected 'HELLO' from tunnel, got: '{received}'",
    );

    // Drop writer and reader to allow the tunnel tasks to finish
    drop(tunnel.writer.take());
    drop(reader);
    tunnel.wait().await;
}
