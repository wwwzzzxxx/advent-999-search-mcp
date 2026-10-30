#!/bin/bash
# Build the Linux release binaries for advent-999-search-mcp.
#
# Run this ON r7525 (Ubuntu 24.04 x86_64, Docker available). It produces:
#   target/release/advent-999-search-mcp                              (x86_64)
#   target/aarch64-unknown-linux-gnu/release/advent-999-search-mcp    (aarch64)
#
# Then copy both back to the Windows checkout as:
#   target/release/advent-999-search-mcp-linux-x86_64
#   target/release/advent-999-search-mcp-linux-aarch64
#
# The aarch64 image is a custom Ubuntu 24.04 image: the stock cross-rs image is
# Ubuntu 16.04 (glibc 2.23, no OpenSSL) and produces a binary that needs an
# ancient libssl.
set -euo pipefail

REPO="${1:-$PWD}"
IMAGE=advent-arm64-cross:24.04
cd "$REPO"

# ---------- x86_64 ----------
echo "==> x86_64"
# shellcheck disable=SC1090
source ~/.cargo/env
cargo build --release
ls -l target/release/advent-999-search-mcp

# ---------- aarch64 image ----------
echo "==> aarch64 cross image"
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  docker rm -f armcross >/dev/null 2>&1 || true
  docker run --name armcross --network host \
    -e http_proxy="${HTTP_PROXY:-}" -e https_proxy="${HTTPS_PROXY:-}" \
    ubuntu:24.04 bash -c '
set -e
rm -f /etc/apt/sources.list.d/*
cat > /etc/apt/sources.list <<EOF
deb [arch=amd64] http://archive.ubuntu.com/ubuntu noble main restricted universe multiverse
deb [arch=amd64] http://archive.ubuntu.com/ubuntu noble-updates main restricted universe multiverse
deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports noble main restricted universe multiverse
deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports noble-updates main restricted universe multiverse
EOF
dpkg --add-architecture arm64
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  gcc-aarch64-linux-gnu libssl-dev:arm64 libc6-dev:arm64 pkg-config build-essential
'
  docker commit armcross "$IMAGE"
  docker rm -f armcross
fi

# ---------- aarch64 build ----------
echo "==> aarch64"
rustup target add aarch64-unknown-linux-gnu >/dev/null
docker run --rm --network host \
  -v "$REPO:/project" \
  -v "$HOME/.rustup:/root/.rustup" \
  -v "$HOME/.cargo:/root/.cargo" \
  -w /project \
  -e RUSTUP_HOME=/root/.rustup \
  -e CARGO_HOME=/root/.cargo \
  -e CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-B/usr/lib/aarch64-linux-gnu -C link-arg=-L/usr/lib/aarch64-linux-gnu" \
  -e PKG_CONFIG_ALLOW_CROSS=1 \
  -e PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig \
  "$IMAGE" bash -c 'export PATH=/root/.cargo/bin:$PATH; cargo build --release --target aarch64-unknown-linux-gnu'

# Bring the aarch64 binary out of the root-owned target dir and make it readable.
docker run --rm \
  -v "$REPO/target/aarch64-unknown-linux-gnu/release:/src" \
  -v "$REPO/target/release:/out" \
  "$IMAGE" bash -c "cp /src/advent-999-search-mcp /out/advent-999-search-mcp-linux-aarch64 && chmod 755 /out/advent-999-search-mcp-linux-aarch64"

cp target/release/advent-999-search-mcp target/release/advent-999-search-mcp-linux-x86_64
echo "==> done"
ls -l target/release/advent-999-search-mcp-linux-x86_64 target/release/advent-999-search-mcp-linux-aarch64
