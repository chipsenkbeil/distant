# Publishing Releases

Guide to publishing the binary and associated crates.

## Version Lifecycle

```
Development:  X.Y.Z-dev              ← master always has a -dev suffix
Nightly:      X.Y.Z-nightly.YYYYMMDD ← CI-stamped at build time, never committed
Release:      X.Y.Z                  ← scripts/set-version.sh X.Y.Z, commit & tag
Post-release: X.Y+1.0-dev            ← scripts/set-version.sh X.Y+1.0-dev, commit & push
```

## Publish Order

Crates must be published in dependency order:

1. `distant-core`
2. `distant-docker`
3. `distant-host`
4. `distant-ssh`
5. `distant` (root binary)

## Full Release Workflow

### 1. Update Changelog

Edit `docs/CHANGELOG.md`:

1. Change `[Unreleased]` to the release version and date: `[X.Y.Z] - YYYY-MM-DD`
2. Add a new `[Unreleased]` header above it
3. At the bottom, add a comparison link for the new version
4. Update the `[Unreleased]` comparison link

### 2. Set the Release Version

```bash
scripts/set-version.sh X.Y.Z
```

This updates `[workspace.package]` version and all `[workspace.dependencies]`
version pins in the root `Cargo.toml`.

### 3. Dry Run

```bash
scripts/dry-run-publish.sh
```

Runs `cargo publish --dry-run` for each crate in dependency order. Verifies
that all crates are publishable.

### 4. Commit, Tag, and Push

```bash
git add -A
git commit -m "Release vX.Y.Z"
git tag vX.Y.Z
git push && git push --tags
```

Pushing the tag triggers the release CI workflow, which builds binaries for all
platforms and creates a GitHub release with the changelog entry.

### 5. Publish to crates.io

```bash
cargo publish --all-features -p distant-core
cargo publish --all-features -p distant-docker
cargo publish --all-features -p distant-host
cargo publish --all-features -p distant-ssh
cargo publish --all-features
```

Wait a few seconds between each crate for crates.io indexing.

### 6. Bump to Next Dev Version

```bash
scripts/set-version.sh X.Y+1.0-dev
git add -A
git commit -m "Bump to X.Y+1.0-dev"
git push
```

## Pre-release Versions

For alpha/beta/rc releases, use a prerelease suffix:

```bash
scripts/set-version.sh 0.22.0-alpha.1
git commit -am "Release v0.22.0-alpha.1"
git tag v0.22.0-alpha.1
git push && git push --tags
```

The release workflow marks tags with prerelease suffixes as pre-releases
on GitHub.

## Nightly Builds

A cron job runs daily at midnight UTC (`.github/workflows/nightly.yml`).
If `master` has new commits since the last nightly, it force-updates the
`nightly` tag, which triggers the release workflow. The version is stamped as
`X.Y.Z-nightly.YYYYMMDD` in the built binaries (never committed to the repo).

## Publish Guard

The `scripts/check-publish-version.sh` script prevents accidental publishing
of `-dev` or other prerelease versions. It reads the workspace version and
aborts if it contains a prerelease suffix.

`scripts/dry-run-publish.sh` runs this guard automatically before attempting
the dry run.
