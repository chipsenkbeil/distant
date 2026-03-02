//! Integration tests for distant Client operations through the Docker backend.

use std::path::PathBuf;

use distant_core::protocol::{
    FileType, SearchQueryCondition, SearchQueryMatch, SearchQueryOptions,
};
use distant_core::{ChannelExt, Client};
use distant_test_harness::docker::{Ctx, client};
use distant_test_harness::skip_if_no_docker;
use rstest::*;
use test_log::test;

/// Returns a temp directory path appropriate for the container OS.
///
/// On Windows, uses `C:\temp` instead of `C:\Windows\Temp` because nanoserver's
/// `ContainerUser` cannot write to system directories via exec commands.
/// The test harness pre-creates `C:\temp` via the tar API during container setup.
fn test_temp_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\temp")
    } else {
        PathBuf::from("/tmp")
    }
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
        .write_file(path.clone(), data.to_vec())
        .await
        .unwrap();
    let result = client.read_file(path.clone()).await.unwrap();
    assert_eq!(result, data);

    let _ = client.remove(path, false).await;
}

#[rstest]
#[test(tokio::test)]
async fn read_file_should_fail_if_missing(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let path = test_temp_dir().join("distant-test-nonexistent-file.txt");
    let result = client.read_file(path).await;
    assert!(result.is_err());
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
        .write_file(path.clone(), b"hello".to_vec())
        .await
        .unwrap();
    client
        .append_file(path.clone(), b" world".to_vec())
        .await
        .unwrap();

    let result = client.read_file(path.clone()).await.unwrap();
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
        .write_file(dir.join("file1.txt"), b"a".to_vec())
        .await
        .unwrap();
    client
        .write_file(dir.join("file2.txt"), b"b".to_vec())
        .await
        .unwrap();

    let (entries, _errors) = client
        .read_dir(dir.clone(), 0, false, false, false)
        .await
        .unwrap();

    let names: Vec<String> = entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
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
        .write_file(src.clone(), b"copy me".to_vec())
        .await
        .unwrap();
    client.copy(src.clone(), dst.clone()).await.unwrap();

    let result = client.read_file(dst.clone()).await.unwrap();
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
        .write_file(src.clone(), b"rename me".to_vec())
        .await
        .unwrap();
    client.rename(src.clone(), dst.clone()).await.unwrap();

    let exists_src = client.exists(src).await.unwrap();
    assert!(!exists_src, "Source should not exist after rename");

    let result = client.read_file(dst.clone()).await.unwrap();
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
        .write_file(path.clone(), b"metadata test".to_vec())
        .await
        .unwrap();

    let metadata = client.metadata(path.clone(), false, false).await.unwrap();
    assert_eq!(metadata.file_type, FileType::File);
    assert!(metadata.len > 0);

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
        .write_file(path.clone(), b"remove me".to_vec())
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
        .write_file(sub.join("file.txt"), b"nested".to_vec())
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

    let cmd = if cfg!(windows) {
        "cmd /c echo hello"
    } else {
        "echo hello"
    };
    let proc = client
        .spawn(cmd.to_string(), Default::default(), None, None)
        .await
        .unwrap();

    // Wait briefly for the process to produce output
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The process should have an ID
    assert!(proc.id() > 0);
}

// ---------------------------------------------------------------------------
// System info
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn system_info_should_return_valid_data(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let info = client.system_info().await.unwrap();

    assert!(!info.family.is_empty());
    assert!(!info.os.is_empty());
    assert!(!info.arch.is_empty());
    assert!(!info.username.is_empty());
    assert!(!info.shell.is_empty());
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

#[rstest]
#[test(tokio::test)]
async fn version_should_include_capabilities(#[future] client: Option<Ctx<Client>>) {
    let mut client = skip_if_no_docker!(client.await);
    let version = client.version().await.unwrap();

    assert!(version.server_version.major > 0 || version.server_version.minor > 0);
    assert!(!version.capabilities.is_empty());
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
    assert!(
        found
            .iter()
            .any(|m| matches!(m, SearchQueryMatch::Contents(c) if c.path == file))
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
    client.write_file(file, b"content".to_vec()).await.unwrap();

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
    assert!(found
        .iter()
        .any(|m| matches!(m, SearchQueryMatch::Path(p) if p.path.to_string_lossy().contains("unique-needle-file"))));

    let _ = client.remove(dir, true).await;
}
