#!/usr/bin/env bash
# Stage a relocatable /usr-style payload; never writes directly to the host root.
set -euo pipefail
if [[ $# != 2 ]]; then echo "Usage: $0 BINARY DESTDIR" >&2; exit 2; fi
TIDE_PACKAGE_SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TIDE_PACKAGE_BINARY=$(realpath -- "$1")
mkdir -p -- "$2"
TIDE_PACKAGE_DEST=$(realpath -- "$2")
if [[ "$TIDE_PACKAGE_DEST" == / ]]; then echo 'Refusing to stage into /' >&2; exit 2; fi
install -Dm755 -- "$TIDE_PACKAGE_BINARY" "$TIDE_PACKAGE_DEST/usr/bin/tide-terminal"
for command in T_: A_:; do
    install -Dm755 -- "$TIDE_PACKAGE_SOURCE/$command" "$TIDE_PACKAGE_DEST/usr/bin/$command"
done
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/packaging/tide-terminal.desktop" "$TIDE_PACKAGE_DEST/usr/share/applications/tide-terminal.desktop"
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/assets/branding/tide-icon.svg" "$TIDE_PACKAGE_DEST/usr/share/icons/hicolor/scalable/apps/tide-terminal.svg"
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/assets/branding/window-icon.png" "$TIDE_PACKAGE_DEST/usr/share/icons/hicolor/128x128/apps/tide-terminal.png"
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/packaging/io.github.ksu_gitreaper807.tide.metainfo.xml" "$TIDE_PACKAGE_DEST/usr/share/metainfo/io.github.ksu_gitreaper807.tide.metainfo.xml"
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/packaging/tide-terminal.1" "$TIDE_PACKAGE_DEST/usr/share/man/man1/tide-terminal.1"
for document in README.md FEATURES.md config.example.toml; do
    install -Dm644 -- "$TIDE_PACKAGE_SOURCE/$document" "$TIDE_PACKAGE_DEST/usr/share/doc/tide-terminal/$document"
done
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/packaging/README.md" "$TIDE_PACKAGE_DEST/usr/share/doc/tide-terminal/packaging/README.md"
install -Dm644 -- "$TIDE_PACKAGE_SOURCE/LICENSE" "$TIDE_PACKAGE_DEST/usr/share/licenses/tide-terminal/LICENSE"
mkdir -p -- "$TIDE_PACKAGE_DEST/usr/share/tide-terminal/vendor"
tar -C "$TIDE_PACKAGE_SOURCE/vendor" --exclude=cache.d --exclude=benchmark.ksh -cf - blesh zsh-autosuggestions README.md |
    tar -C "$TIDE_PACKAGE_DEST/usr/share/tide-terminal/vendor" -xf -
for notice in "$TIDE_PACKAGE_SOURCE"/assets/fonts/*LICENSE* "$TIDE_PACKAGE_SOURCE"/vendor/egui-winit/LICENSE*; do
    install -Dm644 -- "$notice" "$TIDE_PACKAGE_DEST/usr/share/licenses/tide-terminal/$(basename -- "$notice")"
done
printf 'Staged Tide in %s/usr\n' "$TIDE_PACKAGE_DEST"
