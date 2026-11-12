**Tide Terminal** is a native Rust terminal for Linux, with Bash, Zsh, and Fish integration, inline suggestions, input alerts, animated cursor movement, and timer/output rules for owned command runs.

The application defaults to a surface-compatible **integrated GPU**, preferring Vulkan. You can explicitly switch using **Settings → Renderer → Save**, then restart, or override one launch:

```bash
./run.sh gui --gpu integrated
./run.sh gui --gpu discrete
./run.sh gui --gpu auto
./run.sh gui --gpu software
```

Automatic prefers integrated, then discrete, virtual and CPU adapters. The other modes require their specified adapter class and report an error if unavailable. Software mode requires a CPU graphics driver such as Mesa llvmpipe; choosing it does not install that driver. The active adapter is shown in Settings. Existing configurations keep the integrated default. This policy is local to Tide.

Run the included Linux build from this directory:

```bash
./run.sh
./run.sh gui --shell /usr/bin/fish
./run.sh gui --shell /bin/zsh
./run.sh gui --clean --shell /bin/bash
```

`--clean` skips the normal user startup configuration for the selected shell. The application normally loads your startup configuration and then its integration. It does not edit your shell startup files. Keep `vendor/` with the application: Bash suggestions use the bundled ble.sh runtime and Zsh suggestions use zsh-autosuggestions. Fish keeps its native editor. Enhanced suggestions can be disabled in Settings if they conflict with a shell framework.

**Interface.** Tide uses a single compact tab bar with horizontal overflow, a quiet directory/status strip, and separate proportional UI and monospace terminal fonts. Run, History, and Settings open as centered dialogs. Active tabs stay visible when the window is resized, and dialog actions remain accessible at the 640×420 minimum size. Appearance changes are immediate; Save changes persists them.

Each tab shows the last command launched in that terminal, including its arguments, and retains the title after success, failure or interruption. A fresh shell starts with its shell name. Long titles are shortened in the tab; hover to read the full command and working directory. Commands remain isolated to their own tabs.

**Using the window.** Type directly into the terminal. Right arrow accepts an inline suggestion; Tab invokes the shell's own completion. Drag to select text and use Ctrl+Shift+C to copy. Ctrl+Shift+V pastes, with bracketed paste when the active program supports it. Programs requesting mouse reporting receive terminal mouse events; hold Shift to select terminal text instead.

| Shortcut | Action |
|---|---|
| Ctrl+Shift+T | Open another shell tab |
| Ctrl+Tab | Switch tabs |
| Ctrl+Shift+W | Close the current tab and its session |
| Ctrl+Shift+H | Search shared command history |
| Ctrl+Shift+R | Open Run command |
| Ctrl+Shift+A | Append an action to the selected tab's current or most recent command |
| Ctrl+Shift+, | Appearance and behavior settings |
| Ctrl-X, then Ctrl-H | Shell widget: replace the editing buffer with the first matching shared-history command |

History selection inserts a command into the real shell's editing buffer. It does not execute it. The integration also reserves Ctrl-X Ctrl-Y for the application's history-insertion widget. A new terminal tab is the supported way to change shells with full integration; an arbitrarily launched nested shell may only expose its native protocol capabilities.

**Terminal control keys.** Ctrl+A–Z and the legacy control-punctuation keys are forwarded to the PTY. The running shell or program decides their meaning. Typical behavior is:

| Keys | Usual action |
|---|---|
| Ctrl+C | Interrupt the foreground command; cancel the current input line |
| Ctrl+Z | Suspend the foreground job; use `fg` to resume it |
| Ctrl+backslash | Quit the foreground command (SIGQUIT) |
| Ctrl+D | End input on an empty line; a shell may exit |
| Ctrl+S / Ctrl+Q | Pause / resume terminal output when software flow control is enabled |
| Ctrl+U / Ctrl+K / Ctrl+W | Erase to the start / end of the line / erase a word |
| Ctrl+A / Ctrl+E / Ctrl+B / Ctrl+F | Move to the start / end / backward / forward |
| Ctrl+P / Ctrl+N / Ctrl+R | Previous / next history entry / history search |
| Ctrl+L | Redraw or clear the screen |
| Ctrl+V / Ctrl+X | Quote the next character / start an editor key sequence |
| Ctrl+[ / Ctrl+] / Ctrl+^ / Ctrl+_ | ESC / group separator / record separator / unit separator |
| Ctrl+Space or Ctrl+@ / Ctrl+? | NUL / DEL |

Ctrl+Shift+C/V remain clipboard copy/paste. Plain Ctrl+C/X/V keep their terminal meanings even when text is selected or the clipboard is empty. Dialog text fields keep ordinary Ctrl+C/X/V editing. Font size can be changed in Settings; Ctrl+minus and Ctrl+_ are available to terminal programs.

Dialog placeholders and guidance use translucent text; placeholders explain what to enter and disappear when you type. Run focuses Command when the dialog is ready for input. Tab moves through the fields. Invalid durations or patterns are shown inside the form so you can correct them without dismissing a second dialog.

Settings control live background opacity, font size, and smooth cursor motion. Default shell, enhanced suggestions, privacy, and desktop-notification choices apply to newly launched sessions. Opacity needs compositor support; on a display without compositing, a transparent window may appear against a dark background.

The embedded font stack includes the complete Symbols Nerd Font Mono 3.5.1 icon catalogue for eza, shell prompts, Powerline and developer tools; DejaVu Sans for Braille plots and borders; and Noto Sans Symbols 1/2 for additional Unicode symbols. Recognized private-use icons are fitted to their terminal cells to avoid overlapping filenames. No separate font installation is needed. Font attribution, licenses, versions and hashes are in `assets/fonts/`.

**Running with rules.** Choose **Run**, enter a command, and optionally set:

- A total runtime limit: enter a number and select seconds, minutes, or hours. For example, `30` with seconds selected. Explicit durations such as `20m`, `30s`, and `500ms` also work and override the selector.
- A visible-output inactivity limit, using the same unit selector or an explicit duration such as `5m`.
- A literal output marker, such as `TRAINING_COMPLETE`.
- A regular expression, such as `FATAL:.*`.

The first matching condition requests SIGINT for that owned command's process group. If enabled, a three-second cleanup period is followed by SIGKILL. The Stop button uses the same supervisor. The status records the reason, the exit result, and a signal description when applicable. A matched output marker does not manufacture a successful exit status.

Run displays the shell path it will use. The shell comes from Settings → Default shell; a `gui --shell /path/to/shell` launch option overrides it for the current app session. With no saved setting, Tide defaults to `$SHELL`, or `/bin/bash` if unset. Running `exec fish` in a tab does not change the app’s default shell.

Managed runs use a fresh instance of that shell with `-c`, in their own terminal. They inherit the application's environment and the selected terminal's current directory. They do not automatically inherit aliases, functions, or unexported variables from an existing interactive shell. Normal interactive commands keep the real shell's normal execution and job-control behavior; automatic termination is available through managed runs.

The same engine can supervise an executable from another terminal:

```bash
./run.sh run --timeout 20m --grace 3s -- python3 train.py
./run.sh run --stop-on TRAINING_COMPLETE -- python3 -u train.py
./run.sh run --regex 'FATAL:.*' --idle 5m -- ./worker
./run.sh run --json --timeout 10s -- ./worker
./run.sh run --no-force --timeout 30s -- ./worker
```

**Timer command shorthand.** New Tide Bash, Zsh and Fish sessions provide `T_:`:

```bash
T_: 'grep -i "hi" .' : -s 1200
T_: 'python3 -u train.py' : -m 20 -rex '[0-9]+'
T_: './worker' : -idle-s 30 -text 'FINISHED'
T_: 'sleep 60' : -s 10 -grace 500ms
T_: -s 10 -- sleep 60
T_: --help
```

Use outer single quotes around a shell command so its inner quotes, variables, pipes and redirects reach the managed shell. The `:` between the command and rules is optional. `T_:` itself includes its trailing colon and must be followed by a space.

| Flag | Effect |
|---|---|
| `-s 1200` / `-m 20` | Total runtime in seconds / minutes; use one; decimals accepted |
| `-idle-s 30` / `-idle-m 5` | Stop after no visible output for this many seconds / minutes; use one |
| `-rex '[0-9]+'` | Stop when visible output matches a Rust regular expression |
| `-text 'FINISHED'` | Stop when visible output contains this exact, case-sensitive text |
| `-grace 3s` | Time after SIGINT before SIGKILL; defaults to three seconds |
| `--no-force` | Send SIGINT only; commands ignoring it can continue running |
| `--shell /bin/bash` | Choose the shell for a quoted command |
| `--private` / `--notify` / `--json` | Disable Tide history / enable desktop input alerts / emit JSON events |

Rules combine with OR: the first condition reached stops the command. Output rules observe the combined PTY output; they do not filter what you see. With no rules, the command runs until completion or manual interruption. The existing process-group supervisor handles interruption, escalation, password input and exit status.

Quoted commands run in a fresh `shell -c` in the caller's current directory, inheriting exported environment variables. They use `--shell` if supplied, otherwise the launching Tide tab's shell (`TIDE_SHELL`), then Tide's saved Default shell, then `$SHELL` or `/bin/bash`. Interactive aliases, functions and unexported variables are not copied. `-- CMD ARGS...` executes an argv directly and cannot be combined with a quoted command or `--shell`. A private Tide session also makes its `T_:` runs private; your shell's own history is governed by that shell's settings.

Outside Tide, run `./T_: 'sleep 60' : -s 10` from this directory, or `./run.sh timer 'sleep 60' : -s 10`. To make the launcher available in other terminals, add this application's directory to your `PATH`. Restart Tide and open a new tab after updating to load the function; existing tabs keep their original integration.

Use `--notify` on the CLI to enable desktop attention notifications and `--private` to avoid recording that run in Tide history. The CLI uses a real PTY, so stdout and stderr are combined and ordinary terminal echo can be present. It is an interactive supervisor rather than a binary-clean pipe replacement. Interactive password entry uses raw outer-terminal input and restores the original terminal settings afterward. JSON mode is intended for machine consumption.

**Append after starting a command.** Select its tab and click **Append** or press **Ctrl+Shift+A**. This uses the current command, or the most recent tracked command when the tab is at its prompt. It never reruns the original command.

- **`&& After success`**: enter the next command. It runs only after the source exits with status zero. A failed or interrupted source skips it.
- **`| Process captured output`**: enter a filter such as `grep -i include`, `sort`, or `wc -l`. After the source finishes, its retained terminal text is supplied to the new command as finite stdin. Source failure does not discard its output.
- **`>> Append output to file`**: enter a file path, without shell quotes. Existing contents are preserved; a missing file is created. Relative paths use the source command's recorded directory. Parent directories must already exist. Paths are treated literally, without glob, variable or command expansion.

For example, after running `grep -i -r "include" /some/path`, open Append, choose `|`, and enter `sort -u`; or choose `>>` and enter `include-results.txt`. A running source queues the action until completion; an already finished source can be processed immediately. Closing the source tab cancels its waiting action. Reopen Append on that source to cancel a queued action explicitly. One action can wait per source; append another step from the resulting command's new tab.

Capture is bounded terminal text (up to 1 MiB), with ANSI controls removed and progress-line rewrites normalized. It combines stdout/stderr and may be truncated; the dialog and activity status identify truncation. This does not retroactively install a live or binary-clean shell pipe. Only the current/most recent tracked command is available; old command blocks and output from before updating Tide are not recovered. Full-screen applications may not produce a useful text capture.

Follow-up commands open in a fresh instance of the source shell with its recorded directory, inheriting the application's exported environment. Interactive aliases/functions and unexported variables are not copied. They have their own normal completion/Stop lifecycle; the source command's timer is not restarted or transferred. Captures remain in memory until processing; pipe stages use a private temporary stdin file removed when the stage ends. Private source commands make their follow-up runs private too. `>>` writes only when explicitly submitted.

**Append from the prompt with `A_:`.** After an ordinary command finishes:

```sh
printf 'Hello\nbye\n'
A_: '| grep -i "hello"'
A_: '>> results.txt'
A_: '>> "results with spaces.txt"'
A_: '&& echo "Previous command succeeded"'
```

The shell function is installed in new Bash, Zsh and Fish tabs. The source is never rerun. Repeated `A_:` actions reuse the same last completed ordinary command; the follow-up's output and confirmations do not replace it. Another ordinary command becomes the new source. CLI actions run in the same tab; a pipe stage uses the source shell and recorded directory, receives finite captured stdin, and returns its own exit status. `&&` skips a failed source and returns that status. `>>` preserves existing bytes, creates absent files, and reports write errors. Quote filenames with spaces inside the action; file paths are literal without shell expansion. `./A_:` also works within an integrated Tide shell.

New sessions also retain stdout and stderr independently, each up to 1 MiB of normalized terminal text:

```sh
A_: '1>> stdout.txt'
A_: '2>> errors.txt'
A_: '1| grep -i result'
A_: '2| grep -i error'
```

No source wrapper is needed: run the original command normally, then use `A_:` at its next prompt. Bare `|` and `>>` retain their combined-output behavior. Independent stream limits and truncation notices apply to the selected stream. Old sessions/recordings cannot retroactively gain separate streams; restart Tide after updating.

This is not binary-safe or a live pipe. The channels reflect the original program's redirections: if it used `2>&1`, that output arrives on the stdout channel. Writes directly to `/dev/tty` and terminal input echo also arrive on the controlling-terminal/stdout channel; they cannot be identified as stderr. stderr is a separate TTY with synchronized dimensions, but is not the controlling terminal. Programs requiring stderr itself to be the controlling terminal can use `2>/dev/tty`, which intentionally merges that output again. Cross-stream ordering in combined capture is the order Tide receives it, not a guaranteed syscall order. Background output may overlap foreground captures.

Capture remains in memory behind a private per-session socket; pipe input uses an unlinked temporary file. Both stream boundaries must complete before a recording is published. `2>&1 test.txt` does not name a redirection destination; use `2>> test.txt` for the retained stderr channel.

**Input alerts.** Common password/passphrase, fingerprint-reader, confirmation, and “press Enter” prompts produce an amber tab badge and an in-terminal message after a short debounce. Desktop alerts use `notify-send` when enabled and available. Generic matching is suppressed on alternate screens, so an editor displaying the text `Password:` does not automatically trigger an alert. Password keystrokes are forwarded to the program and are not added to Tide history or its notification text.

Fingerprint detection recognizes English instructions such as “Place your finger on the fingerprint reader,” named-finger/thumb variants, and swipe/scan/touch prompts. Newline-terminated instructions stay eligible while the program waits. Visible progress, password fallback, or command completion updates or clears attention. Tide observes terminal text; the program and PAM handle the actual fingerprint scan.

Detection is heuristic unless a cooperative program explicitly reports an input request. Silence alone does not mean the program wants input. A program can explicitly report a request with:

```python
print("\033]777;tide;input;\a", end="", flush=True)
```

An alert is dismissed when input resumes, the prompt changes, or the command settles. A new prompt can alert again. Tide does not answer prompts automatically.

**Local data.** Preferences are stored in `~/.tide.toml` (or the file selected by `TIDE_CONFIG`), and shared command history in `$XDG_DATA_HOME/tide-terminal/history.sqlite3`, using the usual `~/.config` and `~/.local/share` defaults. Leading-space commands are excluded from Tide history. Private sessions skip Tide history and disable the shell history file for app-launched shells. Terminal scrollback stays in memory. Transcript/event recording only occurs when explicitly requested through the testing/debug flags.

**Building and testing.** The included source was tested with Rust 1.99.1. Build dependencies include a C compiler for bundled SQLite, pkg-config, and the Linux graphics/window libraries required by winit/wgpu. Running the GUI requires access to a supported integrated GPU and a Wayland or X11 display.

```bash
cargo build --release --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

The expanded [test-library guide](tests/README.md) maps coverage and known gaps. [Testing resources](tests/resources.md) links the upstream terminal suites and Rust testing tools researched for this project. Run all local automated tiers with `python3 scripts/test_library.py --binary "$PWD/bin/tide-terminal" --output-dir /path/to/results --work-dir /path/to/scratch`; add `--gui` only on an isolated accelerated test display. The JSON report distinguishes successful regression checks, tracked dependency gaps and checks that were not run.

The PTY tests use the installed Bash, Zsh, Fish, and Python 3 executables. `scripts/cli_smoke.py` tests the actual executable and raw terminal restoration using only the Python standard library. `scripts/timer_smoke.py` covers the `T_:` syntax, unit conversions, output rules, shell selection, validation, privacy and standalone launcher. `scripts/gui_smoke.py` drives the actual X11 window using XTest, python-xlib, and Pillow. It runs with isolated config/history/cache and records GPU identity, native GPU screenshots, cursor animation frames, and actual process events. Layout checks cover tab overflow and the 640×420 minimum window size. The default GUI tier requires an accelerated X11/Xwayland display with DRI3. Hosted CI explicitly selects `--gpu software` under Xvfb; that validates GUI behavior without claiming iGPU hardware correctness.

```bash
python scripts/cli_smoke.py --binary "$PWD/bin/tide-terminal" \
  --work-dir /path/to/scratch --report /path/to/cli-results.json

python scripts/timer_smoke.py --binary "$PWD/bin/tide-terminal" --launcher "$PWD/T_:" \
  --work-dir /path/to/scratch --report /path/to/timer-results.json

python scripts/gui_smoke.py --binary "$PWD/bin/tide-terminal" \
  --work-dir /path/to/scratch --output-dir /path/to/gui-results
```

The packaged build was also exercised in a separate KWin virtual Wayland compositor with accelerated Xwayland. Recent KWin versions restrict XTest injection. For that setup, `scripts/kwin_test_input.c` supplies test input through KWin's fake-input protocol; it connects exclusively to the dedicated `tide-qa-test` socket. Generate its protocol header/source from the system `plasma-wayland-protocols/fake-input.xml` using `wayland-scanner`, link against `wayland-client`, and pass the compiled helper through `--kwin-input-helper`. Start the compositor with `--virtual --xwayland --socket tide-qa-test` and launch the GUI test as its `--exit-with-session` command, so the test inherits the compositor's X11 display. This helper is only a test utility; the application itself does not use it.

**Implementation boundaries.** The terminal grid/parser comes from alacritty_terminal; portable-pty owns terminal transport; egui/eframe provide the native UI and wgpu rendering. Application state, rules, attention detection, process supervision, and history are Rust. No C++ application component is required. Shell integration uses shell scripts. SQLite and system graphics libraries are native dependencies.

The app uses bounded queues and output context. Paste messages are limited to 1 MiB; an unresponsive input consumer is reported instead of blocking rule enforcement. Output matching keeps a bounded normalized text window, supports split markers and common color/line-rewrite sequences, and is not a semantic interpreter for every full-screen application's UI. Diagnostic output events are best effort under overload; the terminal parser and rule engine consume the actual PTY stream. Runtime budgets include system suspend and continue while input is requested. On normal run exit, pending stop actions are discarded with that run.

This version targets local Linux sessions. SSH termination does not prove a remote process stopped. Process groups cover ordinary descendants but do not contain children that deliberately create another session/group. It does not implement cgroup containment, a persistent supervisor daemon, crash recovery, tmux pane attribution, saved named rule profiles, the complete Kitty graphics/keyboard protocols, or macOS/Windows ports. Keep the app running while supervising commands.

Bundled shell-plugin versions and licenses are listed in `vendor/README.md`.

To include a real btop rendering check in the GUI smoke suite, pass `--btop-binary /usr/bin/btop`. Add `--btop-only` for a focused check. It uses a temporary CPU/memory-only btop configuration, so process command lines are not recorded in its screenshots or event log.

Pass `--eza-binary /usr/bin/eza` to check real file/folder icons and a gallery of all Nerd Font families. Add `--icons-only` for a focused run. The listing uses synthetic files in a temporary directory. Rust tests audit the full Nerd Font, eza and bundled Unicode symbol inventories and check private-use icon sizing at several supported font sizes.

**Development and CI.** GitHub Actions runs formatting, strict Clippy, all Rust/PTY/property tests and CLI/timer integration on pushes and pull requests. It saves test results as artifacts and explicitly reports that physical iGPU GUI tests are not run on hosted runners. Run the local GUI tier before publishing transport/input/rendering changes. See [FEATURES.md](FEATURES.md) for the feature list and [tests/README.md](tests/README.md) for validation commands.

**Command metadata and trust.** New sessions use a fresh, random session token in versioned OSC 777 envelopes for command labels, cwd, start/end and append markers. Plain OSC 133/7 or old Tide markers printed by a child are not automation authority. A missing, malformed, negative, out-of-range or disagreeing stream status never authorizes `&&`; unavailable captures produce an error. If an interactive shell exits without completing both command boundaries, its process exit does not supply the missing command status. Managed runs use the child process result.

The token prevents accidental output and other/remote sessions from impersonating these hooks. It is **not a security boundary against a hostile local child**: descendants inherit it and can read it. Cooperative input-request messages remain advisory. Arbitrarily nested shells/SSH/tmux do not automatically gain trusted integration; use an app-launched supported shell for these operations. Restart after upgrading to load the new hooks.

**Preferences dotfile.** GPU mode, terminal font file/size, opacity, cursor animation, shell, suggestions, privacy and notifications share one editable TOML file: `~/.tide.toml`. See [config.example.toml](config.example.toml). `TIDE_CONFIG=/absolute/path/profile.toml ./run.sh` selects an alternate profile. An explicit profile never falls back to another file if absent. Paths inside TOML are literal; use an absolute font path rather than `~` or `$HOME`.

If the dotfile is absent, Tide reads the existing `$XDG_CONFIG_HOME/tide-terminal/config.toml` (normally `~/.config/tide-terminal/config.toml`). Saving writes the dotfile atomically with owner-only permissions and leaves that legacy file intact. The dotfile takes precedence; an invalid dotfile reports an error rather than silently loading another profile. To roll back, move the dotfile aside and the legacy settings become available again. Font-file and GPU changes require restarting; other appearance controls retain their live behavior. A missing/invalid custom font falls back visibly to the default without removing icon/Unicode fallback fonts. GUI Save validates the font before replacing a working config.

**Pager input with separate streams.** Programs such as less 710 may open `ttyname(2)` for keyboard input. Tide routes input/replies to the secondary terminal when it alone switches to noncanonical or no-echo mode; input is never duplicated into both terminals. Signal-generating keys still use the controlling terminal. Both-raw or unusual terminal-ownership arrangements remain part of the separate-stderr compatibility limit; `2>/dev/tty` remains an explicit merged-output workaround.
