#!/usr/bin/env bash

# NOTE: Run me from the root of the project! Not within the scripts directory!

###############################################################################
# TOOLCHAIN SETUP
#
# See: https://github.com/messense/homebrew-macos-cross-toolchains
###############################################################################

# For x86_64-unknown-linux-gnu
export CC_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-gcc
export CXX_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-g++
export AR_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-ar
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-unknown-linux-gnu-gcc

# For x86_64-unknown-linux-musl
export CC_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-gcc
export CXX_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-g++
export AR_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-ar
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-unknown-linux-musl-gcc

pushd () {
    command pushd "$@" > /dev/null
}

popd () {
    command popd "$@" > /dev/null
}

###############################################################################
# TARGET GENERATION FOR DISTANT BIN
#
# Note: This is running on an M1 Mac and expects tooling like `lipo`
###############################################################################

TARGET_DIR="./target"
PACKAGE_DIR="${TARGET_DIR}/package"

rm -rf "${PACKAGE_DIR}"
mkdir -p "${PACKAGE_DIR}"

# Apple x86-64 on M1 Mac
TARGET="x86_64-apple-darwin"
echo "Building ${TARGET} distant binary"
cargo build --release --target "${TARGET}"
strip "${TARGET_DIR}/${TARGET}/release/distant"

# Apple ARM on M1 Mac
TARGET="aarch64-apple-darwin"
echo "Building ${TARGET} distant binary"
cargo build --release --target "${TARGET}"
strip "${TARGET_DIR}/${TARGET}/release/distant"

# Combine Mac executables into universal binary
echo "Combining Mac x86 and arm into universal binary"
lipo -create \
    -output "${PACKAGE_DIR}/distant-macos" \
    "${TARGET_DIR}/x86_64-apple-darwin/release/distant" \
    "${TARGET_DIR}/aarch64-apple-darwin/release/distant"

# Linux x86-64 (libc) on M1 Mac
TARGET="x86_64-unknown-linux-gnu"
echo "Building ${TARGET} distant binary"
cargo build --release --target "${TARGET}"
cp "${TARGET_DIR}/${TARGET}/release/distant" "${PACKAGE_DIR}/distant-linux64-gnu"
x86_64-unknown-linux-musl-strip "${PACKAGE_DIR}/distant-linux64-gnu"

# Linux x86-64 (musl) on M1 Mac
TARGET="x86_64-unknown-linux-musl"
echo "Building ${TARGET} distant binary"
cargo build --release --target "${TARGET}"
cp "${TARGET_DIR}/${TARGET}/release/distant" "${PACKAGE_DIR}/distant-linux64-musl"
x86_64-unknown-linux-musl-strip "${PACKAGE_DIR}/distant-linux64-musl"

###############################################################################
# EXECUTABLE PERMISSIONS
###############################################################################

pushd "${PACKAGE_DIR}";
for bin in *; do
    echo "Marking ${bin} executable"
    chmod +x "${bin}"
done
popd

###############################################################################
# TARGET GENERATION FOR DISTANT LUA MODULE
###############################################################################

# Apple x86-64 on M1 Mac
TARGETS=(
    "x86_64-apple-darwin" 
    "aarch64-apple-darwin" 
    "x86_64-unknown-linux-gnu" 
)
pushd "distant-lua";
for TARGET in "${TARGETS[@]}"; do
    echo "Building ${TARGET} for Lua"
    cargo build --release --target "${TARGET}"

    if [ "$TARGET" == "x86_64-apple-darwin" ]; then
        cp "${TARGET_DIR}/${TARGET}/release/libdistant_lua.dylib" "${PACKAGE_DIR}/distant_lua-macos-x86_64.so"
    elif [ "$TARGET" == "aarch64-apple-darwin" ]; then
        cp "${TARGET_DIR}/${TARGET}/release/libdistant_lua.dylib" "${PACKAGE_DIR}/distant_lua-macos-aarch64.so"
    else
        cp "${TARGET_DIR}/${TARGET}/release/libdistant_lua.so" "${PACKAGE_DIR}/distant_lua-linux-x86_64.so"
    fi
done
popd

###############################################################################
# SHA 256 GENERATION
#
# Note: This is running on an M1 Mac and expects tooling like `shasum`
###############################################################################

echo "Generating sha256sum index file"
(cd "${PACKAGE_DIR}" && shasum -a 256 * > sha256sum.txt)

pushd "${PACKAGE_DIR}";
for bin in *; do
    if [ "${bin}" != "sha256sum.txt" ]; then
        echo "Generating sha256sum for ${bin}"
        shasum -a 256 "${bin}" > "${bin}.sha256sum"
    fi
done
popd

###############################################################################
# DISPLAY RESULTS
###############################################################################

open "${PACKAGE_DIR}"
