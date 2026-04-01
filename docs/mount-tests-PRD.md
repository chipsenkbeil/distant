# Mount CLI Integration Tests — PRD

## Overview

Implement automated CLI integration tests for `distant mount`, `distant
unmount`, and `distant mount-status` covering all test cases from
`docs/MANUAL_TESTING.md`. Tests exercise every combination of plugin
backend (Host, SSH, Docker) x mount backend (NFS, FUSE, Windows Cloud
Files, macOS FileProvider) that is available on the platform.

## Architecture

### rstest_reuse templates with cfg_attr cases

Add `rstest_reuse` for reusable test case templates. Define two templates:

**`all_plugins`** — for non-mount tests (can replace inline case lists):
```rust
#[export]
#[template]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[cfg_attr(feature = "docker", case::docker(Backend::Docker))]
fn all_plugins(#[case] backend: Backend) {}
```

**`plugin_x_mount`** — every valid plugin x mount combination:
```rust
#[export]
#[template]
#[rstest]
#[cfg_attr(feature = "mount-nfs", case::host_nfs(Backend::Host, MountBackend::Nfs))]
#[cfg_attr(feature = "mount-nfs", case::ssh_nfs(Backend::Ssh, MountBackend::Nfs))]
#[cfg_attr(all(feature = "docker", feature = "mount-nfs"),
    case::docker_nfs(Backend::Docker, MountBackend::Nfs))]
#[cfg_attr(all(feature = "mount-fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")),
    case::host_fuse(Backend::Host, MountBackend::Fuse))]
#[cfg_attr(all(feature = "mount-fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")),
    case::ssh_fuse(Backend::Ssh, MountBackend::Fuse))]
#[cfg_attr(all(feature = "mount-windows-cloud-files", target_os = "windows"),
    case::host_wcf(Backend::Host, MountBackend::WindowsCloudFiles))]
#[cfg_attr(all(feature = "mount-macos-file-provider", target_os = "macos"),
    case::host_fp(Backend::Host, MountBackend::MacosFileProvider))]
fn plugin_x_mount(#[case] backend: Backend, #[case] mount: MountBackend) {}
```

### Test harness additions

**`distant-test-harness/Cargo.toml`:**
- Add `mount = ["dep:distant-mount"]` feature
- Add `distant-mount` optional dep
- Add `rstest_reuse = "0.7"` dep

**`distant-test-harness/src/mount.rs`** (new):
- Re-export `distant_mount::MountBackend`
- `MountProcess` struct: spawn foreground mount, wait for "Mounted",
  canonical path, umount -f before kill, wait_for_unmount polling
- `build_test_app_bundle()` for FileProvider (all in Rust, no scripts)
- Template definitions (`all_plugins`, `plugin_x_mount`)

### Test file organization

```
tests/cli/
  mount/
    mod.rs           — re-exports, shared helpers
    browse.rs        — MNT-01..03
    file_read.rs     — FRD-01..03
    subdirectory.rs  — SDT-01..02
    file_create.rs   — FCR-01..02
    file_delete.rs   — FDL-01..02
    file_rename.rs   — FRN-01..02
    file_modify.rs   — FMD-01..02
    directory_ops.rs — DOP-01..03
    readonly.rs      — RDO-01..03
    remote_root.rs   — RRT-01..02
    multi_mount.rs   — MML-01..03
    status.rs        — MST-01..03
    unmount.rs       — UMT-01..03
    edge_cases.rs    — EDG-01..05
    daemon.rs        — DMN-01
    backend/
      mod.rs                 — backend-specific test module
      nfs.rs                 — BKE-NFS-*
      fuse.rs                — BKE-FUSE-*
      macos_file_provider.rs — FP-01..04
      windows_cloud_files.rs — BKE-WCF-*
```

### Test pattern

```rust
use rstest_reuse::apply;
use distant_test_harness::mount::plugin_x_mount;

#[apply(plugin_x_mount)]
#[test_log::test]
fn mount_should_list_root_directory(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mount-browse");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &[
        "--remote-root", &dir,
    ]);

    let entries = std::fs::read_dir(mp.mount_point()).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert!(entries.contains(&"hello.txt".to_string()));
}
```

## Phases

### Phase 1: Harness + Templates
- Add `mount` feature + `distant-mount` dep to harness
- Add `rstest_reuse` dep
- Create `mount.rs` with MountBackend re-export, MountProcess, templates
- Wire into workspace Cargo.toml

### Phase 2: Rewrite Core Tests (MNT, FRD, SDT)
- Rewrite browse.rs, file_read.rs, subdirectory.rs with `#[apply(plugin_x_mount)]`
- Use `BackendCtx` + `cli_write/read/exists/mkdir`

### Phase 3: Rewrite Write Tests (FCR, FDL, FRN, FMD, DOP)
- Rewrite all write tests with template

### Phase 4: Rewrite Management Tests (RDO, RRT, MML, MST, UMT)
- Rewrite readonly, remote_root, multi_mount, status, unmount

### Phase 5: Edge Cases + Daemon + Backend-Specific
- edge_cases.rs, daemon.rs
- backend/nfs.rs, backend/fuse.rs, backend/macos_file_provider.rs,
  backend/windows_cloud_files.rs

## Non-Goals

- Stress testing
- Performance benchmarking
