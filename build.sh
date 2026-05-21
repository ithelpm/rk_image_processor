#!/usr/bin/env bash
# Cross-compile rk_image_processor for aarch64-linux-gnu inside Docker.
#
# Usage:
#   ./build.sh           # build (incremental if build-aarch64/ exists)
#   ./build.sh clean     # wipe build-aarch64/ then rebuild from scratch

set -euo pipefail

IMAGE="rk3588-builder"
DIR="$(cd "$(dirname "$0")" && pwd)"

if [ "${1-}" = "clean" ]; then
    echo "[build.sh] Cleaning build-aarch64/ ..."
    rm -rf "$DIR/build-aarch64"
fi

# Build the toolchain image (cached after the first run; re-run automatically
# picks up any Dockerfile changes thanks to the layer cache).
docker build -t "$IMAGE" "$DIR"

# Run the cross-compile.  The project root (source + aarch64_sysroot/) is
# mounted read-write so the build output lands directly on the host.
docker run --rm -v "$DIR":/workspace "$IMAGE"

echo ""
echo "Done: build-aarch64/rk3588_demo (aarch64-linux-gnu)"
