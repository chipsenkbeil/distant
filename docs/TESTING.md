# Testing

This document covers testing practices, infrastructure, and CI configuration
for the distant project.

## Test Tiers

### Unit Tests

Located inline in source files (`#[cfg(test)]` modules). Test individual
functions and types in isolation.

```bash
cargo test --all-features -p distant-core
```

### Integration Tests

Located in `tests/` directories within each crate. Test interactions between
components, often requiring real infrastructure (sshd, Docker).

```bash
cargo test --all-features -p distant-ssh
cargo test --all-features -p distant-docker
```

### CLI Tests

Located in `tests/` under the root crate. Test the `distant` binary end-to-end
using `assert_cmd` and `portable-pty`.

```bash
cargo test --all-features --test '*'
```

## Running Tests

### All Tests

```bash
# Standard cargo test
cargo test --all-features --workspace

# With nextest (preferred — parallel execution, better output)
cargo nextest run --all-features --workspace

# CI profile (retries, slow-timeout)
cargo nextest run --profile ci --all-features --workspace
```

### Individual Crates

```bash
cargo test --all-features -p distant-core
cargo test --all-features -p distant-docker
cargo test --all-features -p distant-host
cargo test --all-features -p distant-ssh
```

### Single Test

```bash
cargo test --all-features -p <package> <test_name>
```

### Doc Tests

```bash
cargo test --all-features --workspace --doc
```

## Test Infrastructure

### SSH (`distant-test-harness/src/sshd.rs`)

Integration tests in `distant-ssh` spawn real `sshd` instances per-test on
random high ports. The test harness:

- Generates temporary host keys and identity keys
- Writes per-test `sshd_config` files
- Spawns `sshd` in foreground mode
- Cleans up on drop

Tests use `rstest` fixtures that provide a `Ctx<SshClient>` or similar context
with a connected client.

### Docker (`distant-test-harness/src/docker.rs`)

Integration tests in `distant-docker` use real Docker containers. The harness:

- Creates containers from `ubuntu:22.04` with `sleep infinity` as entrypoint
- Tests use the `skip_if_no_docker!` macro to skip gracefully when Docker is unavailable
- Containers are cleaned up on drop

### CLI Test Context Types (`distant-test-harness/src/manager.rs`)

CLI integration tests use context types that manage the full lifecycle of
distant processes (manager, server, connections):

| Context Type | Backend | How It Connects |
|-------------|---------|----------------|
| `HostManagerCtx` | Host (local) | `distant connect distant://...` |
| `ManagerOnlyCtx` | None (manager only) | No connection — for testing error paths |
| `SshManagerCtx` | SSH plugin | `distant connect ssh://localhost:{port}` via per-test sshd |
| `SshLaunchCtx` | SSH plugin | `distant launch ssh://127.0.0.1:{port}` via per-test sshd |
| `DockerManagerCtx` | Docker plugin | `distant connect docker://...` via ephemeral container |

> **Note:** There is no `DockerLaunchCtx` because Docker does not support the
> `distant launch` workflow — containers are connected to directly.

All context types expose `new_assert_cmd()`, `new_std_cmd()`, and `cmd_parts()`
to build commands pre-configured with the correct socket, log file, and
connection ID.

### Cross-Plugin Parity Testing (`tests/cli/parity.rs`)

The `BackendCtx` enum (`distant-test-harness/src/backend.rs`) wraps all context
types behind a single interface. Tests use rstest `#[case]` to run the same
assertion across Host, SSH, and Docker backends:

```rust
#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
fn fs_read_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    // ...test logic using ctx.new_assert_cmd(["fs", "read"])...
}
```

The `skip_if_no_backend!` macro skips gracefully when a backend's
prerequisites are unavailable (no sshd, no Docker).

### Tunnel Testing (`tests/cli/tunnel.rs`)

Tunnel tests use a custom `tcp-echo-server` binary
(`distant-test-harness/src/bin/tcp_echo_server.rs`) instead of platform-specific
`nc`/netcat. The server binds to `127.0.0.1:0`, prints its port to stdout,
accepts one connection, echoes all data back, and exits on EOF or timeout.

### PTY / Predictive Echo Testing (`tests/cli/pty.rs`)

PTY tests are cross-platform and use `portable-pty` (`PtySession` in
`tests/cli/pty.rs`) to interact with `distant shell`, `distant spawn --pty`,
and `distant ssh` (which also allocates a PTY). All PTY tests use rstest
multi-backend (Host, SSH, Docker) via `BackendCtx`. On Windows, `PtySession`
automatically handles ConPTY cursor position queries (`\x1b[6n`) to prevent
I/O deadlocks. Purpose-built binaries exercise different PTY scenarios:

- `pty-echo`: byte-by-byte stdin→stdout echo loop
- `pty-interactive`: mini-shell with `$ ` prompt, `exit`, `passwd`, Ctrl+C handling
- `pty-password`: password prompt with echo disabled (rpassword), then echo loop

Tests verify `--predict off` and `--predict on` modes work end-to-end. Platform-
specific commands (e.g., `sh -c` vs `cmd /c`, `stty size` vs `mode con`, `tput`
vs PowerShell ANSI sequences) use `#[cfg]` for behavioral dispatch — the same
test runs on all platforms with appropriate command variants.

## Nextest Configuration

Configuration lives in `.config/nextest.toml`.

### Test Groups (Throttling)

To prevent resource exhaustion, certain test categories have thread limits:

| Group | Scope | Max Threads | Reason |
|-------|-------|-------------|--------|
| `ssh-integration` | `distant-ssh` lib + SSH CLI tests | 4 | Prevents sshd fork exhaustion |
| `ssh-integration-windows` | `distant-ssh` lib (Windows) | 1 | Windows sshd is fragile |
| `docker-integration` | `distant-docker` lib | 2 | Prevents Docker API contention |
| `tunnel-tests` | `test(tunnel_)` | 4 | Prevents port exhaustion |
| `service-tests` | `test(service_)` | 1 | Service install/uninstall is sequential |

### CI Profile

The `ci` profile adds:

- **Retries**: 4 (handles intermittent SSH/Docker failures)
- **Slow timeout**: 60s period, terminate after 3 periods (180s total)

## CI Configuration

CI runs on three platforms via `.github/workflows/ci.yml`:

| Platform | Rust | Notes |
|----------|------|-------|
| `ubuntu-latest` | stable | Pre-pulls Docker image (`ubuntu:22.04`), creates `/run/sshd` |
| `macos-latest` | stable | |
| `windows-latest` | stable | Stops system sshd, configures firewall for high ports |
| `ubuntu-latest` | 1.88.0 | MSRV validation |

### Platform-specific Setup

**Linux**: `sudo mkdir -p /run/sshd` (required by sshd) and `docker pull ubuntu:22.04`.

**Windows**: Stops the system `sshd` service (conflicts with per-test instances),
enables `ssh-agent`, and opens firewall ports 49152–65535. Windows tests get
extra nextest retries (3) and a 90s test timeout.

## Writing Tests

### Naming

Test modules use descriptive names matching the function under test. Individual
tests describe the behavior being verified:

```rust
#[cfg(test)]
mod my_function_tests {
    #[test]
    fn returns_error_when_path_does_not_exist() { ... }

    #[test]
    fn succeeds_with_valid_input() { ... }
}
```

### Fixtures

Use `rstest` for parameterized tests and shared fixtures:

```rust
use rstest::*;

#[fixture]
fn ctx() -> Ctx<Client> {
    // Set up test context
}

#[rstest]
fn read_file_should_return_contents(ctx: Ctx<Client>) {
    // Test using the shared context
}
```

### Error Cases

Every public function should have tests for both success and error paths.
Don't assume error cases are "obvious" — test them explicitly.

### Test Quality

- Never dismiss test failures as "intermittent" without investigation
- Every failure must be analyzed for root cause
- Prefer `assert_eq!` and `unwrap()` over `assert!(result.is_ok())` — validate the
  value inside Ok, not just success. When exact values are unpredictable, use
  `assert!` with descriptive messages explaining what was expected

### Test Organization

- **No separator comments**: Do not use `// --- section ---` or similar
  dividers in test modules. Test function names provide sufficient organization.
- **Flat test structure**: Prefer flat test functions with descriptive names
  over nested test modules. Use `<subject>_should_<behavior>` naming.
  Nested modules are acceptable only when they share substantial setup code
  (fixtures, helper functions) that would be awkward at the top level.
  Never suffix nested modules with `_tests`.
- **Helper method coverage**: Every helper function (public, `pub(crate)`,
  or private) must have unit tests covering each code path. When functions
  depend on external types that can't be constructed in tests (e.g., network
  handles), introduce a zero-cost trait abstraction and use a mock
  implementation.
