use indoc::indoc;
use rstest::*;

use crate::cli::fixtures::*;

const EXPECTED_TABLE: &str = indoc! {"
+------------------+------------------------------------------------------------------+
| kind             | description                                                      |
+------------------+------------------------------------------------------------------+
| cancel_search    | Supports canceling an active search against the filesystem       |
+------------------+------------------------------------------------------------------+
| capabilities     | Supports retrieving capabilities                                 |
+------------------+------------------------------------------------------------------+
| copy             | Supports copying files, directories, and symlinks                |
+------------------+------------------------------------------------------------------+
| dir_create       | Supports creating directory                                      |
+------------------+------------------------------------------------------------------+
| dir_read         | Supports reading directory                                       |
+------------------+------------------------------------------------------------------+
| exists           | Supports checking if a path exists                               |
+------------------+------------------------------------------------------------------+
| file_append      | Supports appending to binary file                                |
+------------------+------------------------------------------------------------------+
| file_append_text | Supports appending to text file                                  |
+------------------+------------------------------------------------------------------+
| file_read        | Supports reading binary file                                     |
+------------------+------------------------------------------------------------------+
| file_read_text   | Supports reading text file                                       |
+------------------+------------------------------------------------------------------+
| file_write       | Supports writing binary file                                     |
+------------------+------------------------------------------------------------------+
| file_write_text  | Supports writing text file                                       |
+------------------+------------------------------------------------------------------+
| metadata         | Supports retrieving metadata about a file, directory, or symlink |
+------------------+------------------------------------------------------------------+
| proc_kill        | Supports killing a spawned process                               |
+------------------+------------------------------------------------------------------+
| proc_resize_pty  | Supports resizing the pty of a spawned process                   |
+------------------+------------------------------------------------------------------+
| proc_spawn       | Supports spawning a process                                      |
+------------------+------------------------------------------------------------------+
| proc_stdin       | Supports sending stdin to a spawned process                      |
+------------------+------------------------------------------------------------------+
| remove           | Supports removing files, directories, and symlinks               |
+------------------+------------------------------------------------------------------+
| rename           | Supports renaming files, directories, and symlinks               |
+------------------+------------------------------------------------------------------+
| search           | Supports searching filesystem using queries                      |
+------------------+------------------------------------------------------------------+
| system_info      | Supports retrieving system information                           |
+------------------+------------------------------------------------------------------+
| unwatch          | Supports unwatching filesystem for changes                       |
+------------------+------------------------------------------------------------------+
| watch            | Supports watching filesystem for changes                         |
+------------------+------------------------------------------------------------------+
"};

#[rstest]
#[test_log::test]
fn should_output_capabilities(ctx: DistantManagerCtx) {
    ctx.cmd("capabilities")
        .assert()
        .success()
        .stdout(EXPECTED_TABLE)
        .stderr("");
}
