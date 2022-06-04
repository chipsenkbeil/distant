use crate::stress::fixtures::*;
use assert_fs::prelude::*;
use distant_core::{ChangeKindSet, SessionChannelExt};
use rstest::*;

const MAX_FILES: usize = 500;

#[rstest]
#[tokio::test]
#[ignore]
async fn should_handle_large_volume_of_file_watching(#[future] ctx: DistantSessionCtx) {
    let ctx = ctx.await;
    let mut channel = ctx.session.clone_channel();

    let tenant = "watch-stress-test";
    let root = assert_fs::TempDir::new().unwrap();

    let mut files_and_watchers = Vec::new();

    for n in 1..=MAX_FILES {
        let file = root.child(format!("test-file-{}", n));
        eprintln!("Generating {:?}", file.path());
        file.touch().unwrap();

        eprintln!("Watching {:?}", file.path());
        let watcher = channel
            .watch(
                tenant,
                file.path(),
                false,
                ChangeKindSet::modify_set(),
                ChangeKindSet::empty(),
            )
            .await
            .unwrap();

        eprintln!("Now watching file {}", n);
        files_and_watchers.push((file, watcher));
    }

    for (file, _watcher) in files_and_watchers.iter() {
        eprintln!("Updating {:?}", file.path());
        file.write_str("updated text").unwrap();
    }

    for (file, watcher) in files_and_watchers.iter_mut() {
        eprintln!("Checking {:?} for changes", file.path());
        match watcher.next().await {
            Some(_) => {}
            None => panic!("File {:?} did not have a change detected", file.path()),
        }
    }
}
