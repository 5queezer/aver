#!/usr/bin/env bash
# Reproduce the linux-x86_64 release build (.github/workflows/release.yml) locally and
# install the result into a clean container with no Rust toolchain.
# Usage: scripts/release-dry-run.sh [OUT_DIR]   (env: ENGINE=podman|docker, IMAGE=ubuntu:24.04)
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-${TMPDIR:-/tmp}/aver-release-dryrun}"
IMAGE="${IMAGE:-ubuntu:24.04}"
SUFFIX=linux-x86_64
ENGINE="${ENGINE:-$(command -v podman || command -v docker || true)}"

[ "$(uname -s)-$(uname -m)" = "Linux-x86_64" ] || { echo "dry run is linux-x86_64 only" >&2; exit 1; }

# Build job: same flags, strip and tarball layout as release.yml
cargo build --release --locked -p aver-cli -p aver-server
rm -rf "$OUT" && mkdir -p "$OUT/stage" "$OUT/dist"
for bin in aver aver-server; do
  cp "target/release/$bin" "$OUT/stage/$bin"
  strip "$OUT/stage/$bin"
  tar -C "$OUT/stage" -czf "$OUT/dist/$bin-$SUFFIX.tar.gz" "$bin"
done

# Publish job: unit + checksums, same command as the workflow
cp contrib/systemd/aver-mcp.service "$OUT/dist/aver-mcp.service"
(cd "$OUT/dist" && sha256sum -- *.tar.gz aver-mcp.service > SHA256SUMS && cat SHA256SUMS)

if [ -z "$ENGINE" ]; then
  echo "NOTE: no podman/docker found; container install check NOT executed." >&2
  exit 0
fi

# Clean-container check: no cargo; python3 only to serve the directory over loopback http
"$ENGINE" run --rm \
  -v "$OUT/dist:/release:ro,Z" -v "$PWD/install.sh:/install.sh:ro,Z" \
  -v "$PWD/scripts/container-verify.sh:/verify.sh:ro,Z" \
  "$IMAGE" bash /verify.sh
