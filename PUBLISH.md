# Publish

Guide to publishing the binary and associated crates.

## 1. Update Changelog

Ensure that the changelog is updated for a new release. The CI build requires
that the release version is specified in the format: `[VERSION] - DATE`.

1. Update the changelog by changing `[Unreleased]` to the latest version and
   date.
2. Re-add a new `[Unreleased]` header at the top.
3. At the bottom, add a new link for the current version.
4. Update the `[Unreleased]` link with the latest tag.

## 2. Update Crate Versions

Run a command to update the crate versions. An easy way is to use `sed`.

On Mac, this would be `sed -i '' "s~0.17.4~0.17.5~g" **/*.toml` where the old
and new versions would be specified.

*Make sure to review the changed files! Sometimes a version overlaps with
another crate and then we've bumped something wrong!*

## 3. Build to get Cargo.lock update

Run `cargo build` to get a new `Cargo.lock` refresh and commit it.

## 4. Tag Commit

Tag the release commit with the form `vMAJOR.MINOR.PATCH` by using 
`git tag vMAJOR.MINOR.PATCH` and publish the tag via `git push --tags`.

Once the tag is pushed, a new job will start to build and publish the artifacts
on Github.

## 5. Publish Crates

Now, `cd` into each sub-crate and publish. Sometimes, it takes a little while
for a crate to be indexed after getting published. This can lead to the publish
of a downstream crate to fail. If so, try again in a couple of seconds.

1. **distant-net:** `(cd distant-net && cargo publish)`
2. **distant-core:** `(cd distant-core && cargo publish)`
3. **distant-ssh2:** `(cd distant-ssh2 && cargo publish)`
4. **distant:** `cargo publish`

## 6. Celebrate

Another release done!
