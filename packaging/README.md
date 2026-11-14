# Linux distribution

The native package targets x86-64 Arch Linux and CachyOS. It installs the compiled app, shell helpers, bundled integrations, icons, desktop/AppStream metadata, a man page, documentation and license notices. Running it needs no checkout or Rust toolchain.

## Build and install

From a checkout with its source files tracked in Git:

```bash
bash packaging/build-arch.sh /absolute/path/to/package-output
cd /absolute/path/to/package-output
sha256sum -c SHA256SUMS
sudo pacman -U ./tide-terminal-0.1.0-6-x86_64.pkg.tar.zst
tide-terminal
```

The output contains the native package, a concrete `PKGBUILD` and its checksummed source archive. Only Git-tracked files enter the archive. The build uses locked Cargo dependencies and generic x86-64 CPU flags, including for SQLite, rather than CachyOS's host-specific `-march=native`. `check()` runs all Rust targets sequentially. Install base-devel, Cargo, pkgconf and the runtime/check dependencies declared by the PKGBUILD; makepkg checks these first. Additional makepkg options may follow the output directory.

This is an unsigned local package, not an Arch/AUR publication. Checksums detect changes; they do not establish publisher identity. A rolling-Arch build does not establish compatibility with older glibc distributions. Install the GPU driver for the machine on which Tide will run.

The package installs `/usr/bin/tide-terminal`, `/usr/bin/T_:` and `/usr/bin/A_:`, with its shell runtime in `/usr/share/tide-terminal/vendor/`. Desktop entries, icons, AppStream metadata, manuals, documentation and licenses use their standard `/usr/share` locations. Preferences stay in `~/.tide.toml`; upgrades do not overwrite them. Uninstall with `sudo pacman -R tide-terminal`.

The build sets `TIDE_PACKAGED=1` to omit the development-checkout asset fallback from the release executable. It clears Tide's release artifacts before compiling, retaining dependency caches while preventing normalized archive timestamps from hiding source changes in a reused target directory. Runtime resources are resolved relative to the installed binary, so the payload can also be relocated as a whole. The source build's plugin tests explicitly use their source fixtures.

## Desktop integration

```bash
tide-terminal --working-directory '/path/with spaces'
tide-terminal -e python3 script.py 'argument with spaces'
tide-terminal --hold -e bash -c 'printf "hello\n"'
```

`-e` executes an argv directly; shell syntax requires an explicit shell. Its terminal closes on completion unless `--hold` is set. Other tabs remain open, and existing close protection still applies to live jobs. These options also work after `gui`; other GUI options retain their existing `gui --option` form.

The desktop entry supplies the command, directory and hold keys used by [xdg-terminal-exec](https://github.com/Vladimir-csp/xdg-terminal-exec). KDE's [KTerminalLauncherJob](https://api.kde.org/kterminallauncherjob.html) appends `-e` for commands and supplies the working directory.

After installation, select Tide in KDE's Default Applications → Terminal Emulator, or run:

```bash
kwriteconfig6 --file kdeglobals --group General --key TerminalApplication --notify tide-terminal
kwriteconfig6 --file kdeglobals --group General --key TerminalService --notify tide-terminal.desktop
```

For xdg-terminal-exec, put `tide-terminal.desktop` first in `~/.config/xdg-terminals.list` (or `kde-xdg-terminals.list`). Package installation does not change defaults automatically. Applications with their own explicit terminal preference may need it changed separately. The login shell is unchanged.

## Validation

`scripts/package_smoke.py` checks the actual payload, standalone helpers, licenses, directory errors and desktop entry. Its GUI tier checks exact arguments, cwd, automatic close, `--hold` and Bash/Zsh/Fish startup. Bash/Zsh must load plugins from the relocated installation rather than the checkout. Use a dedicated test display:

```bash
bash packaging/stage.sh /path/to/tide-terminal '/tmp/relocated package'
python3 scripts/package_smoke.py --gui --prefix '/tmp/relocated package/usr' \
  --work-dir /tmp/tide-package-work --report /tmp/tide-package-results.json
```

Hosted CI stages into a path containing a space and explicitly tests software rendering. Local checks establish iGPU operation separately. Inspect the native artifact with `namcap PKGBUILD` and `namcap package.pkg.tar.zst`; runtime-loaded graphics/input libraries cannot be inferred from ELF links alone.
