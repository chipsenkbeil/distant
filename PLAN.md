You are an expert on Windows Cloud Files API

# Resources to become an expert

1. Guide to cloud sync engine that supports placeholder files:
   https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/cfApi/build-a-cloud-file-sync-engine.md
2. General list of markdown files about cloud files:
   https://github.com/MicrosoftDocs/win32/tree/docs/desktop-src/cfApi
3. Formal page with references about cloud sync engines:
   https://learn.microsoft.com/en-us/windows/win32/cfapi/cloud-files-api-portal
4. Cloud mirror example:
   https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine#cloud-mirror-sample
   and sample code
   https://github.com/Microsoft/Windows-classic-samples/tree/main/Samples/CloudMirror

# Overview

You are reviewing the attempt that is not working (failing with 0x8007017C) to
integrate the cloud files api into the distant-mount library such that you can
run `distant mount --backend windows-cloud-files ...` and mount a distant-backed
filesystem onto Windows. The attempt was made within
`distant-mount/src/backend/windows_cloud_files.rs` and has not successfully
worked. It has been using a very experimental `cloud-filter` crate version 0.0.6
that may not actually work (unclear). Your mission is to rewrite this to not use
the `cloud-filter` crate and instead directly make use of the `windows` crate
(we're on 0.58, but we can bump up to 0.62.2 or something inbetween) to reduce
reliance on an experimental crate. This is similar to how we've directly used
the objc2 Rust bindings for the macos file provider.

# Mission

Implement a working version of the cloud files api that lets us mount, unmount,
and check status of the windows cloud files api layer that is built on top of
distant. So, for example, if you do `distant connect ssh://example.com` and then
do `distant mount --backend windows-cloud-files C:\Users\senkwich\CloudMount` it
should use the cloud files api to provide the `CloudMount` folder within
`C:\Users\senkwich` that has the files from the cwd of `example.com`.

Steps are:

1. Review the above numbered resources to familiarize yourself with the cloud
   files api and what it takes to build the sync engine in general, the concepts
   themselves, etc.
2. Example the previous attempt that was using the distant api underneath to
   power the cloud filters implementation
3. See how we would build this out using the windows crate directly instead of
   needing the cloud-filters crate
4. Ensure that this runs async using either the RemoteFs directly or via the
   Runtime abstraction. You should remove attempts that were tried with block_on
   or similar as they never worked. We want to make sure that this does not
   block the main thread.

# Ground work

You should take advantage of a windows 11 vm running that you can access via ssh
through `ssh windows-vm` without a password.

You can run a distant server either within the vm or outside the VM (on the mac
laptop) and connect to it using `distant connect distant://:<key>@<laptop>:<port>`.

Running the distant server can be done in the foreground via `distant server
listen`. Or if you need to background it then `distant server listen --daemon`.

You will need to copy files from the Mac laptop (where you are running as a
agent) to the windows VM by using

```bash
rsync -avz \
    --exclude target/ \
    --exclude .git/ \
    /Users/senkwich/projects/distant/ \
    windows-vm:/cygdrive/c/Users/senkwich/Projects/distant/
```

And then you'd need to do stuff like run `cargo` to build, clippy, etc. and then
run the build at some point, maybe directly from `target/debug/distant.exe`.

From there, you can use `dir` to see if there's anything within the directory
that is created, and read logs if needed.

Never create a commit, always make changes on the mac laptop and copy over to
test.

# Success

In terms of success, you should be able to:
1. Mount a directory on Windows where you
    1. See the root level files of cwd via `dir`
    2. Can create a new file (text file) with some content and it shows up
       on the remote machine
    3. Can delete a file (the text file you just created) and it disappears on
       the remote machine
    4. Traverse within directories of the mount
2. Mount multiple directories using the --remote-root flag and not have them
   clobber each other
3. Mount multiple directories from different connections
4. List active mounts via `distant mount-status` that include the cloud files
   api mounts
5. Unmount a single mounted directory (in the case you mount two via
   --remote-root) without affecting other mounts (including windows cloud files)
   at the same time
6. Unmount all via the `distant unmount --all` and have include the windows
   cloud files mounts be removed
