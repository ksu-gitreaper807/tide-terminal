Tide's test library checks terminal semantics and the product features layered on top of them. It is a local automated regression suite, not a claim of complete VT, xterm, Kitty, Unicode or platform conformance.

Run the automated library from the application directory:

```bash
cargo build --release --locked
# Use the binary you just built, or bin/tide-terminal in the distributed bundle.
python3 scripts/test_library.py --binary "$PWD/bin/tide-terminal" \
  --output-dir /path/to/results --work-dir /path/to/scratch \
  --target-dir /path/to/cargo-target
```

The runner checks formatting and strict Clippy, runs the Rust tests, then runs the existing CLI and timer suites against the explicitly selected binary. `summary.json` records each command, duration, status, binary SHA-256, known gaps, unexecuted checks, and separate corpus/replay counts. A failed or timed-out step produces a nonzero runner result. Checks continue after a failure to preserve the other evidence. The standalone launcher uses the bundle's `bin/tide-terminal`, whose hash is recorded separately if you choose another binary for the CLI tests. Build and copy a changed executable into `bin/` before testing a refreshed bundle.

The default generated-test seed is `20261112`, with 256 trials per property. Choose another seed or more trials using `--seed` and `--property-cases`. Proptest saves minimized failure seeds in `properties.proptest-regressions`. Reproduce the Rust tier alone with:

```bash
PROPTEST_CASES=256 PROPTEST_RNG_SEED=20261112 \
  cargo test --all-targets --locked -- --test-threads=1
cargo test --test terminal_conformance
cargo test --test observer_cases
```

PTY tests run serially in the aggregate runner because interactive completion tests contend for timing under heavy parallel compilation. Their waits have deadlines; a timeout is a failure. There are no automatic retries or silent shell skips. Bash, Zsh, Fish, Python 3, a C compiler, and the project's ordinary Linux build dependencies must be installed. The Rust tier needs no visible window or GPU. The desktop feature remains enabled so font coverage and UI input mapping tests are included.

| Layer | Library | Checks |
|---|---|---|
| Screen semantics | `terminal_conformance.rs`, `cases/terminal.json` | 62 named fixtures: cursor movement/clamping, tabs, hard/soft line breaks, erasure, insertion/deletion, scrolling regions, alternate screen, modes, Unicode width/combining marks, colors/attributes, titles, replies, cancellation and reset |
| Upstream compatibility | `upstream_compat.rs`, `upstream/` | All 45 Alacritty release reference recordings and eight independent libvterm VT scripts, replayed at four chunk sizes (212 scenario replays), plus hash/pin/registration verification; see [provenance and limits](upstream/README.md) |
| Text/protocol observation | `observer_cases.rs`, `cases/observer.json` | 83 named fixtures, each replayed at every possible two-part byte split: password, sudo, PIN, passphrase, fingerprint, confirmation, progress rewrites, editor suppression, OSC 7/133/777, percent/base64 decoding and malformed markers |
| Generated streams | `properties.rs` | Seven Proptest properties: valid-stream chunk equivalence, arbitrary-byte safety and reset recovery, guard chunk equivalence, resize bounds, timer boundaries/one-shot behavior, literal matching, and bounded Unicode lines; also oversized OSC/DCS/PM/APC recovery, corpus registration, and a minimized malformed UTF-8 reproducer |
| Timer rules | `rule_cases.rs`, `src/rules.rs` | Inclusive nanosecond/millisecond boundaries, idle vs runtime, OR/precedence, case sensitivity, literal metacharacters, Unicode/multiline regex, validation limits, one-shot state, grace and force behavior |
| Persistent history | `history_cases.rs`, `src/history.rs`, `privacy.rs` | Literal SQL metacharacters, Unicode/multiline round trips, ranking, deduplication, metadata, prefix-only suggestions, limits, reopening, simultaneous connections, excluded/private commands and startup overrides |
| Copy/selection | `selection_cases.rs` | Forward/reverse ranges, edge clamping, wide/combining characters, hard/soft line breaks and scrolled selection |
| Actual terminal replies | `terminal_queries.rs` | Nine raw-PTY child dialogues check cursor/status/device attributes, text-area size, foreground/background/cursor colors, 256-color palette queries and explicit color overrides |
| Append CLI | `append_cli.rs` | 21 named tests with Bash/Zsh/Fish matrices: 15 output forms, pipeline EOF/chains/exit status/Ctrl+C, repeated file append and failure cases, source preservation/isolation, literal filenames, socket permissions and cleanup; actual CLI executable invoked by shell functions |
| Split output capture | `split_capture.rs`, unit tests in `src/stream_frame.rs` | Five command-boundary/channel-isolation checks, including staggered completions and bounded unmatched commands; four UTF-8/ANSI framing checks prevent fragments from being spliced across channels |
| Appended actions | `appending.rs` | 21 checks: independent stream limits and password privacy, source identity and command boundaries, deferred success/failed-exit gating, captured stdin across Bash/Zsh/Fish, filtering without rerunning, literal file paths, append preservation/errors, truncation, simultaneous stdout/stderr pressure with final markers, password privacy and temporary-input cleanup |
| Shell/process integration | `pty.rs` | Bash/Zsh/Fish syntax/startup hooks, suggestions, history insertion, all standard TTY controls, real password/fingerprint requests, timer/output/idle rules, process-group interruption/escalation, unrelated-process protection, descendants, blocked stdin, resizing, privacy and T_: |
| CLI artifacts | `scripts/cli_smoke.py`, `scripts/timer_smoke.py` | Six existing CLI checks and 12 timer-wrapper checks: actual executable, quoting, units, regex/text, shell choice, exit status, hidden password input, terminal restoration and standalone launcher |
| Rendering and interaction | `scripts/gui_smoke.py`, font tests in `src/theme.rs` | 35-check GUI tier with btop/eza enabled: six dialog appended-action checks and four real-prompt CLI append checks plus Intel iGPU identity, modal typing/validation, keyboard/clipboard, timers/alerts, cursor motion, opacity, overflow/minimum layout, Braille and icon coverage |

Each screen fixture has an independently specified expected screen and cursor. It is not a dump blessed from the current implementation. Four feed strides cover whole, bytewise, three-byte and eleven-byte reads. Observer fixtures use every two-part split, including splits inside UTF-8, escape terminators, base64 and percent-encoding. Generated cases are counted as trials, not separate hand-authored tests. The runtime timer engine and PTY transport are reused in the tests rather than replaced with mocks.

Two dependency gaps are explicit exceptions, kept separate from correct expectations:

- `cases/known-gaps.json`: Alacritty Terminal 0.26.0 leaves the first row uncleared for CSI 1 J when the cursor is on row two. The expected screen remains correct; the test verifies the observed divergence and fails if it changes or starts passing, requiring removal or review of the exception.
- `cases/stream-known-gaps.json`: a four-byte malformed UTF-8 stream split as `[CE] [9B 48 84]` drops an ASCII `H` that a whole-stream feed retains. CAN does not recover that lost character. A later cancel/reset/clear permits subsequent output again. The reproducer checks both observed screens and recovery. Arbitrary invalid bytes therefore have safety/reset assertions; valid generated streams retain strict chunk-equivalence assertions.

No test is marked ignored for these gaps. The runner labels a successful run `passed_with_known_gaps`. A passing Rust process does not mean those two semantics conform. Changing either observed result forces triage; do not add exceptions or update screen expectations just to obtain green results.

The tests exposed and fixed three Tide defects: a literal tab marker was passed to the glyph renderer, indexed/cursor/dynamic color requests were answered with the default foreground color, and process exit could finalize output capture while the reader still had queued bytes. Those reproductions remain in the library. The output-drain regression was reproduced with two bounded CPU competitors sharing one test CPU: the old code finalized after 302 ms with 38 chunks pending and lost the stderr tail. The fix drains pending output before completion. The existing 300 ms grace still releases a quiet inherited PTY, and explicit close can end an active drain. A deterministic drain-policy test checks these boundaries; the real PTY test also writes over 1 MiB on each stream and checks both final markers and independent truncation flags.

To include the GUI tier, run the same aggregate command with `--gui` on a dedicated accelerated X11/Xwayland display. Forward `--kwin-input-helper`, `--btop-binary /usr/bin/btop` and `--eza-binary /usr/bin/eza` as needed. Do not run synthetic input against a working desktop. The existing application README explains the separate KWin test compositor and helper. A plain Xvfb surface cannot validate the integrated-GPU policy. For software GUI checks explicitly use `scripts/gui_smoke.py --gpu software`; those results identify a CPU adapter. Without `--gui`, the JSON explicitly records rendering as not run.

For fixture changes, name the behavior, link the controlling protocol reference or original bug in the review, and specify the desired result before observing the implementation. Use synthetic credentials, paths and process data. Preserve minimized failures as a deterministic regression plus the generator seed. Re-run the responsible layer, then the aggregate runner. GPU screenshots need their own evidence; headless semantic checks do not prove glyph appearance or dialog usability.

External resources and the remaining conformance/quality work are in [resources.md](resources.md). The bundle vendors the selected upstream expectations documented in [upstream/README.md](upstream/README.md). It does not run or claim to pass the complete external vttest, esctest2, Kitty or Konsole suites. The new compatibility tests run automatically in the existing GitHub Actions Rust test job, without downloading upstream repositories during CI.

GitHub Actions (`.github/workflows/ci.yml`) runs the headless gates on every push/PR and uploads results. It also exercises the GUI under Xvfb with an explicitly selected software renderer. That does not establish real integrated/discrete GPU behavior. Local hardware results remain separate from hosted results.

Metadata hardening tests (`metadata.rs`, `split_capture.rs`, and the real Bash/Zsh/Fish append matrix) reject unmarked/wrong-session metadata, unknown/invalid exit codes, channel disagreement and missing completions. They verify that forged output cannot change cwd/command identity or enable a success-only follow-up. `Observer::default()` remains an untrusted protocol fixture parser; production sessions use its token-required constructor.

Coverage-guided fuzzing lives in `fuzz/`. GitHub Actions runs `metadata` and `completion` for 60 seconds each on pushes/PRs/manual runs, and 600 seconds each nightly. cargo-fuzz uses nightly Rust, AddressSanitizer, coverage feedback, debug assertions and overflow checks. Version 0.13.2 and a separate dependency lockfile are recorded. The workflow restores learned corpora and uploads logs, corpora and crash reproducers, including on failure. The existing fixed-seed Proptest suite remains in the ordinary test job.

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly fuzz run metadata -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly fuzz run completion -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
```

`metadata` mutates arbitrary bytes/chunking and asserts that output without the session token cannot supply command authority. Valid UTF-8 also has a marker-equivalence oracle. `completion` generates per-channel status combinations and checks that only two matching valid codes establish a known result. Neither target executes fuzzed shell commands. Fuzzing is bounded evidence, not exhaustive proof or a claim that the two existing core-parser deviations are resolved. Minimize every finding and add a deterministic regression before fixing it; do not weaken an oracle to hide a failure.

Dotfile migration tests cover legacy fallback, dotfile precedence, invalid-file errors, atomic updates, permission 0600, preference preservation and an isolated missing profile. GUI/CLI tests set `TIDE_CONFIG` to a temporary file and never save over the user's dotfile. Custom font files are parsed before replacing the working font stack; defaults and symbol fallbacks remain available.

`pager.rs` covers the exact leaked OSC 10/11 and device-attribute replies, keyboard input on the stderr terminal, Ctrl+C foreground delivery, real less page/arrow keys and quit, and real man pages followed by shell input in Bash/Zsh/Fish. The GUI tier also scrolls a real pager with the wheel, checks changed rendered content, and quits through GUI input. CI installs less/man-db/manpages explicitly.

For a focused real-window pager regression, run `scripts/gui_smoke.py --pager-only` with the normal binary/output/work arguments on an isolated display. The KWin helper emits real wheel-axis events; hosted Xvfb uses XTest wheel buttons. Readiness assertions join PTY output events because a read boundary may split the marker from its newline.
