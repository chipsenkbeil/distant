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
using `assert_cmd` and `expectrl`.

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

## Nextest Configuration

Configuration lives in `.config/nextest.toml`.

### Test Groups (Throttling)

To prevent resource exhaustion, certain crates have thread limits:

| Group | Crate | Max Threads | Reason |
|-------|-------|-------------|--------|
| `ssh-integration` | `distant-ssh` | 4 | Prevents sshd fork exhaustion |
| `docker-integration` | `distant-docker` | 2 | Prevents Docker API contention |

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
fn test_read_file(ctx: Ctx<Client>) {
    // Test using the shared context
}
```

### Error Cases

Every public function should have tests for both success and error paths.
Don't assume error cases are "obvious" — test them explicitly.

### Test Quality

- Never dismiss test failures as "intermittent" without investigation
- Every failure must be analyzed for root cause
- Use `assert!` with descriptive messages: `assert!(result.is_ok(), "expected success but got: {result:?}")`
- Prefer `assert_eq!` over `assert!` when comparing values
