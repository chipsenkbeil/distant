#/usr/bin/env bash

# All crates to publish; NOTE: Order matters here! Publish happens in exact
# order, so depenencies of each other must be published first! dev-dependencies
# are temporarily removed using cargo-hack
CRATES=(
  distant-core
  distant-ssh2
  distant
)

# https://stackoverflow.com/questions/59895/how-can-i-get-the-source-directory-of-a-bash-script-from-within-the-script-itsel
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
CHANGELOG="$DIR/../CHANGELOG.md"
ROOT_CARGO_TOML="$DIR/../Cargo.toml"
PREFIX=distant
DRY_RUN=1
VERBOSE=0
QUIET=0
GIT_BRANCH=master
SKIP_CHANGELOG=0
SKIP_GIT_TAG=0
SKIP_CARGO_TOML_UPDATE=0
SKIP_CARGO_PUBLISH=0
SKIP_GIT_PUSH=0
SKIP_SAFETY_PROMPT=0
WAIT_BETWEEN_PUBLISH=10

# Supports OSX, Ubuntu, Freebsd, Cygwin, CentOS, Red Hat Enterprise, & Msys
# https://izziswift.com/sed-i-command-for-in-place-editing-to-work-with-both-gnu-sed-and-bsd-osx/
sedi () {
  if [ "$VERBOSE" -eq 1 ]; then
    sed "$@" > ".file.tmp.out"
    diff -u "${@: -1}" ".file.tmp.out"
    rm ".file.tmp.out"
  fi

  # Special situation to use sed to return some value rather than modify a file
  if [ "$1" = "{GET}" ]; then
    sed "${@:2}"

  # Otherwise, if not using a dry run, we want to perform the operation
  elif [ "$DRY_RUN" -eq 0 ]; then
    sed --version >/dev/null 2>&1 && sed -i -- "$@" || sed -i "" "$@"
  fi
}

prompt_to_continue () {
  while true; do
    read -p "$* [y/n]: " yn
    case $yn in
      [Yy]*) return 0;;
      [Nn]*) echo "Aborted" ; exit 1;;
    esac
  done
}

print_msg () {
  if [ "$QUIET" -eq 0 ]; then
    echo "$@"
  fi
}

# NOTE: We want to detect the current version without diff
TARGET_VERSION="$(sedi {GET} -n "1,/^version/ s/^version = \"\([^\"]*\)\"/\1/p" "$ROOT_CARGO_TOML")"
NEXT_VERSION=

function usage {
  echo "Usage: $(basename $0) [-vfhq] [-s STEP] [-w SECONDS] [-b BRANCH] [-t TARGET_VERSION] [-n NEXT_VERSION]" 2>&1
  echo "Release the current version of $PREFIX crates."
  echo
  echo "   -t VERSION  Specify target version to use (default = $TARGET_VERSION)"
  echo '   -n VERSION  Specify next version to update all Cargo.toml (does nothing if not provided)'
  echo '   -s STEP     Skips the specified step and can be supplied multiple times'
  echo '               Choices are changelog, git-tag, git-push, cargo-toml-update, cargo-publish, safety-prompt'
  echo "   -w SECONDS  Time to wait between publishing of crates in seconds (default = $WAIT_BETWEEN_PUBLISH)"
  echo "   -b BRANCH   Specify git branch to push to (default = $GIT_BRANCH)"
  echo '   -f          Force release, rather than performing dry run'
  echo '   -h          Print this help information'
  echo '   -v          Increase verbosity'
  echo '   -q          Suppress output (quiet)'
  exit 1
}

while getopts ':vfhqt:n:s:b:w:' arg; do
  case "${arg}" in
    t) TARGET_VERSION=${OPTARG};;
    n) NEXT_VERSION=${OPTARG};;
    q) QUIET=1;;
    v) VERBOSE=1;;
    f) DRY_RUN=0;;
    b) GIT_BRANCH=${OPTARG};;
    w) WAIT_BETWEEN_PUBLISH=${OPTARG};;
    s)
      case "${OPTARG}" in
        changelog) SKIP_CHANGELOG=1;;
        git-tag) SKIP_GIT_TAG=1;;
        git-push) SKIP_GIT_PUSH=1;;
        cargo-toml-update) SKIP_CARGO_TOML_UPDATE=1;;
        cargo-publish) SKIP_CARGO_PUBLISH=1;;
        safety-prompt) SKIP_SAFETY_PROMPT=1;;
        *)
          echo "Unknown step to skip: ${OPTARG}"
          echo
          usage
          ;;
      esac
      ;;
    h)
      usage
      ;;
    :)
      echo "Option missing argument: -${OPTARG}"
      echo
      usage
      ;;
    ?)
      echo "Invalid option: -${OPTARG}"
      echo
      usage
      ;;
  esac
done

shift "$OPTIND"

if [ "$SKIP_SAFETY_PROMPT" -eq 0 ]; then
  echo '!!! SAFETY PROMPT !!!'
  echo
  [ "$DRY_RUN" -eq 0 ] \
    && echo "This is NOT a dry run!" \
    || echo "This is a dry run..."
  echo
  echo "Target Version: $TARGET_VERSION"
  echo "Next Version: $NEXT_VERSION"
  echo "Git Branch: $GIT_BRANCH"
  echo "Wait Between Publish: $WAIT_BETWEEN_PUBLISH"
  echo
  echo "Skip Changelog: $SKIP_CHANGELOG"
  echo "Skip Cargo Toml Update: $SKIP_CARGO_TOML_UPDATE"
  echo "Skip Cargo Publish: $SKIP_CARGO_PUBLISH"
  echo "Skip Git Tag: $SKIP_GIT_TAG"
  echo "Skip Git Push: $SKIP_GIT_PUSH"
  echo
  echo "Dry Run: $DRY_RUN"
  echo "Verbose: $VERBOSE"
  echo "Quiet: $QUIET"
  prompt_to_continue "Really continue?"
fi

# Update the changelog with our new version information
# 1. Replace unreleased with version being published
# 2. Replace release date with actual date in YYYY-MM-DD format
# 3. Add new unreleased template at top of changelog
# 4. Commit all changes in git
if [ "$SKIP_CHANGELOG" -eq 1 ]; then
  print_msg 'Skipping changelog updates!'
elif [ -n "$TARGET_VERSION" ]; then
  print_msg "[$TARGET_VERSION]: $CHANGELOG"
  sedi "s/Unreleased/$TARGET_VERSION/g" "$CHANGELOG"
  sedi "s/ReleaseDate/$(date "+%Y-%m-%d")/g" "$CHANGELOG"
  sedi "s/<!-- next-header -->/<!-- next-header -->\n\n## [Unreleased] - ReleaseDate/g" "$CHANGELOG"

  # If not dry-run, we will add the changelog updates as a new commit
  if [ "$DRY_RUN" -eq 0 ]; then
    git add --all
    git commit -m "[Release $TARGET_VERSION] Changelog updates"
  else
    print_msg 'git add --all'
    print_msg "git commit -m \"[Release $TARGET_VERSION] Changelog updates\""
  fi
else
  print_msg 'Target version not provided! Skipping CHANGELOG.md updates & tagging!'
fi

# Publish each crate with current version
if [ "$SKIP_CARGO_PUBLISH" -eq 0 ]; then
  for crate in "${CRATES[@]}"; do
    print_msg "Publishing $crate"

    dry_run_arg=
    if [ "$DRY_RUN" -eq 1 ]; then
      dry_run_arg=--dry-run
    fi

    quiet_arg=
    if [ "$QUIET" -eq 1 ]; then
      quiet_arg=--quiet
    fi

    cargo hack publish -p "$crate" --no-dev-deps --allow-dirty $dry_run_arg $quiet_arg

    # Wait N seconds to give crates.io a chance to process
    if [ "$WAIT_BETWEEN_PUBLISH" -gt 0 ]; then
      sleep $WAIT_BETWEEN_PUBLISH
    fi
  done
else
  print_msg 'Skipping Cargo crate publishing!'
fi

# Tag the commit that served as our point of publish
if [ "$SKIP_GIT_TAG" -eq 0 ]; then
  if [ "$DRY_RUN" -eq 0 ]; then
    git tag "v$TARGET_VERSION"
  else
    print_msg "git tag \"v$TARGET_VERSION\""
  fi
else
  print_msg 'Skipping git tagging!'
fi

# Update all Cargo.toml with version change for crates
# 1. Replace crate's version with new version
# 2. Replace dependency crates' versions with new version
if [ "$SKIP_CARGO_TOML_UPDATE" -eq 1 ]; then
  print_msg 'Skipping Cargo.toml updates!'
elif [ -n "$NEXT_VERSION" ]; then
  CARGO_TOML_FILES=($(find "$DIR/.." -name "Cargo.toml"))
  for cargo_toml in "${CARGO_TOML_FILES[@]}"; do
    print_msg "[$TARGET_VERSION -> $NEXT_VERSION]: $cargo_toml"
    sedi "1,/^version/ s/^version = \".*\"/version = \"$NEXT_VERSION\"/g" "$cargo_toml"
    sedi "s/^\($PREFIX.*version = \"\)[^\"]*\(\".*\)$/\1=$NEXT_VERSION\2/g" "$cargo_toml"
  done

  # If not dry-run, we will add the Cargo.toml updates as a new commit
  if [ "$DRY_RUN" -eq 0 ]; then
    git add --all
    git commit -m "[Release $TARGET_VERSION] Bump to next version ($NEXT_VERSION)"
  else
    print_msg 'git add --all'
    print_msg "git commit -m \"[Release $TARGET_VERSION] Bump to next version ($NEXT_VERSION)\""
  fi
else
  print_msg 'Next version not provided! Skipping Cargo.toml updates!'
fi

# Push changes and tags to origin
if [ "$DRY_RUN" -eq 0 ]; then
  if [ "$SKIP_GIT_PUSH" -eq 0 ]; then
    git push origin "$GIT_BRANCH"
    if [ "$SKIP_GIT_TAG" -eq 0 ]; then
      git push origin "v$TARGET_VERSION"
    fi
  fi
else
  if [ "$SKIP_GIT_PUSH" -eq 0 ]; then
    print_msg "git push origin \"$GIT_BRANCH\""
    if [ "$SKIP_GIT_TAG" -eq 0 ]; then
      print_msg "git push origin \"v$TARGET_VERSION\""
    fi
  fi
fi
