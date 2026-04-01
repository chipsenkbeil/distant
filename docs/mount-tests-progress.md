# Mount CLI Integration Tests — Progress

> Auto-updated by the `/mount-test-loop` command.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 1: Harness + Templates

- [x] **P1.1** Add mount support to test harness
  - Harness: `mount = ["dep:distant-mount"]` feature + rstest_reuse dep
  - Workspace: extended check-cfg for mount feature names in templates
  - Binary crate: `distant-test-harness` gets `mount` feature + rstest_reuse

- [x] **P1.2** Create `distant-test-harness/src/mount.rs`
  - Re-exports MountBackend from distant_mount
  - MountProcess with spawn/wait/canonical-path/umount/wait_for_unmount
  - build_test_app_bundle() for FileProvider (macOS, all in Rust)
  - all_plugins + plugin_x_mount templates with cfg_attr cases

- [x] **P1.3** Verify templates compile and expand correctly
  - Template defined in tests/cli/mount/mod.rs (binary crate context for cfg_attr)
  - Generates 6 cases on macOS: host_nfs, ssh_nfs, docker_nfs, host_fuse, ssh_fuse, host_fp
  - 5/6 pass (FileProvider needs .app bundle — Phase 5)
  - Template re-exported from harness doesn't work (cfg_attr evaluated at harness compile time)

---

## Phase 2: Core Read Tests

- [x] **P2.1** `browse.rs` — MNT-01, MNT-02, MNT-03 — 15/18 pass (3 FP)
- [x] **P2.2** `file_read.rs` — FRD-01, FRD-02, FRD-03 — 15/18 pass (3 FP)
- [x] **P2.3** `subdirectory.rs` — SDT-01, SDT-02 — 10/12 pass (2 FP)

---

## Phase 3: Write Tests

- [ ] **P3.1** `file_create.rs` — FCR-01, FCR-02
- [ ] **P3.2** `file_delete.rs` — FDL-01, FDL-02
- [ ] **P3.3** `file_rename.rs` — FRN-01, FRN-02
- [ ] **P3.4** `file_modify.rs` — FMD-01, FMD-02
- [ ] **P3.5** `directory_ops.rs` — DOP-01, DOP-02, DOP-03

---

## Phase 4: Mount Management

- [ ] **P4.1** `readonly.rs` — RDO-01, RDO-02, RDO-03
- [ ] **P4.2** `remote_root.rs` — RRT-01, RRT-02
- [ ] **P4.3** `multi_mount.rs` — MML-01, MML-02, MML-03
- [ ] **P4.4** `status.rs` — MST-01, MST-02, MST-03
- [ ] **P4.5** `unmount.rs` — UMT-01, UMT-02, UMT-03

---

## Phase 5: Edge Cases + Daemon + Backend-Specific

- [ ] **P5.1** `edge_cases.rs` — EDG-01..05
- [ ] **P5.2** `daemon.rs` — DMN-01
- [ ] **P5.3** `backend/nfs.rs` — BKE-NFS-*
- [ ] **P5.4** `backend/fuse.rs` — BKE-FUSE-*
- [ ] **P5.5** `backend/macos_file_provider.rs` — FP-01..04
- [ ] **P5.6** `backend/windows_cloud_files.rs` — BKE-WCF-*

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `BackendCtx`
- **Templates:** `all_plugins`, `plugin_x_mount` via rstest_reuse
- **Mount helper:** `MountProcess` in harness mount module
- **Seed data:** `ctx.cli_write()`, `ctx.cli_mkdir()`, `ctx.unique_dir()`
- **Verification:** `ctx.cli_read()`, `ctx.cli_exists()`
- **Run:** `cargo nextest run --all-features -p distant -E 'test(mount::)'`
