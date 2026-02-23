use assert_fs::prelude::*;
use distant_core::ChannelExt;
use rstest::*;
use test_log::test;

use distant_test_harness::host::*;

// 64KB is maximum TCP packet size
const MAX_TCP_PACKET_BYTES: usize = 65535;

// 640KB should be big enough to cause problems
const LARGE_FILE_LEN: usize = MAX_TCP_PACKET_BYTES * 10;

#[rstest]
#[test(tokio::test)]
async fn should_handle_large_files(#[future] ctx: ClientCtx) {
    let ctx = ctx.await;
    let mut channel = ctx.client.clone_channel();

    let root = assert_fs::TempDir::new().unwrap();

    // Generate data
    eprintln!("Creating random data of size: {LARGE_FILE_LEN}");
    let mut data = Vec::with_capacity(LARGE_FILE_LEN);
    for i in 0..LARGE_FILE_LEN {
        data.push(i as u8);
    }

    // Create our large file to read, write, and append
    let file = root.child("large_file.dat");
    eprintln!("Writing random file: {:?}", file.path());
    file.write_binary(&data)
        .expect("Failed to write large file");

    // Perform the read
    eprintln!("Reading file using distant");
    let mut new_data = channel
        .read_file(file.path())
        .await
        .expect("Failed to read large file");
    assert_eq!(new_data, data, "Data mismatch");

    // Perform the write after modifying one byte
    eprintln!("Writing file using distant");
    new_data[LARGE_FILE_LEN - 1] = new_data[LARGE_FILE_LEN - 1].overflowing_add(1).0;
    channel
        .write_file(file.path(), new_data.clone())
        .await
        .expect("Failed to write large file");
    let data = tokio::fs::read(file.path())
        .await
        .expect("Failed to read large file");
    assert_eq!(new_data, data, "Data was not written correctly");

    // Perform append
    eprintln!("Appending to file using distant");
    channel
        .append_file(file.path(), vec![1, 2, 3])
        .await
        .expect("Failed to append to large file");
    let new_data = tokio::fs::read(file.path())
        .await
        .expect("Failed to read large file");
    assert_eq!(new_data[new_data.len() - 3..], [1, 2, 3]);
}
