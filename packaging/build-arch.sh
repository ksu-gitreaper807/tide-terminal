#!/usr/bin/env bash
# Build a native package and a matching, checksummed source distribution.
set -euo pipefail
if [[ $# -lt 1 ]]; then echo "Usage: $0 OUTPUT_DIR [makepkg options...]" >&2; exit 2; fi
TIDE_SOURCE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$TIDE_SOURCE_ROOT" log -1 --format=%ct)}
mkdir -p -- "$1"
TIDE_PACKAGE_OUT=$(realpath -- "$1")
shift
TIDE_PACKAGE_VERSION=$(python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' "$TIDE_SOURCE_ROOT/Cargo.toml")
TIDE_SOURCE_ARCHIVE="$TIDE_PACKAGE_OUT/tide-terminal-$TIDE_PACKAGE_VERSION.tar.gz"
# Only version-controlled paths are eligible; user output and credentials stay out.
git -C "$TIDE_SOURCE_ROOT" ls-files -z |
    tar -C "$TIDE_SOURCE_ROOT" --null -T - --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner --transform="s,^,tide-terminal-$TIDE_PACKAGE_VERSION/," -czf "$TIDE_SOURCE_ARCHIVE"
TIDE_SOURCE_SHA256=$(sha256sum -- "$TIDE_SOURCE_ARCHIVE")
TIDE_SOURCE_SHA256=${TIDE_SOURCE_SHA256%% *}
sed "s/@SOURCE_SHA256@/$TIDE_SOURCE_SHA256/;s/^pkgver=.*/pkgver=$TIDE_PACKAGE_VERSION/" "$TIDE_SOURCE_ROOT/packaging/PKGBUILD.in" > "$TIDE_PACKAGE_OUT/PKGBUILD"
cd -- "$TIDE_PACKAGE_OUT"
makepkg --force --noconfirm "$@"
sha256sum -- PKGBUILD "tide-terminal-$TIDE_PACKAGE_VERSION.tar.gz" tide-terminal-*.pkg.tar.zst > SHA256SUMS
printf 'Native package and source: %s\n' "$TIDE_PACKAGE_OUT"
