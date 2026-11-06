#!/bin/sh
# Build a tetron-browser release zip for linux-amd64: the Carbonyl v0.0.3
# Chromium runtime with libcarbonyl.so rebuilt from this tree on Debian 12
# (glibc 2.36), so the result runs on Debian 12 and later.
# Usage: scripts/release-tetron-browser.sh OUTDIR
# Writes OUTDIR/tetron-browser.linux-amd64.zip and its .sha256. The Chromium
# program "carbonyl" is renamed tetron-browser in the zip.
set -eu
OUT=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
RUST_IMAGE=rust:1.85.1-bookworm
UPSTREAM_URL=https://github.com/fathyb/carbonyl/releases/download/v0.0.3/carbonyl.linux-amd64.zip
UPSTREAM_SHA=46a902ea29bb32f773fb4ee341423705b92345a810422b8df93eaea6a1ec7ad2
WORK="$ROOT/DO-NOT-COMMIT/release-work"
mkdir -p "$OUT" "$WORK"

up="$WORK/carbonyl-v0.0.3.linux-amd64.zip"
[ -f "$up" ] || curl -fsSL -o "$up" "$UPSTREAM_URL"
echo "$UPSTREAM_SHA  $up" | sha256sum -c --quiet

docker run --rm -u "$(id -u):$(id -g)" -e CARGO_HOME=/work/DO-NOT-COMMIT/release-work/cargo-home \
	-v "$ROOT":/work:ro -v "$WORK":/work/DO-NOT-COMMIT/release-work -w /work "$RUST_IMAGE" \
	sh -c 'cargo test --release --locked --target-dir DO-NOT-COMMIT/release-work/target &&
		cargo build --release --locked --target-dir DO-NOT-COMMIT/release-work/target'
lib="$WORK/target/release/libcarbonyl.so"
glibc=$(objdump -T "$lib" | grep -oE 'GLIBC_[0-9.]+' | sort -Vu | tail -1)
echo "libcarbonyl.so needs $glibc"

python3 -I - "$up" "$lib" "$OUT/tetron-browser.linux-amd64.zip" "tetron-browser-$VERSION/" <<'PY'
import sys, zipfile
src, lib, out, new = sys.argv[1:5]
old = "carbonyl-0.0.3/"
date = (2026, 11, 6, 0, 0, 0)  # fixed, so the same inputs give the same zip
zin = zipfile.ZipFile(src)
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zo:
    for i in sorted(zin.infolist(), key=lambda i: i.filename):
        rel = i.filename[len(old):]
        # The Chromium program is named after the product, so ps and htop
        # show tetron-browser; it finds its files next to itself.
        zi = zipfile.ZipInfo(new + ("tetron-browser" if rel == "carbonyl" else rel), date)
        zi.external_attr = i.external_attr
        zi.compress_type = zipfile.ZIP_STORED if i.is_dir() else zipfile.ZIP_DEFLATED
        data = open(lib, "rb").read() if i.filename == old + "libcarbonyl.so" else zin.read(i)
        zo.writestr(zi, data)
PY
(cd "$OUT" && sha256sum tetron-browser.linux-amd64.zip > tetron-browser.linux-amd64.zip.sha256 && cat tetron-browser.linux-amd64.zip.sha256)
