This directory contains pinned third-party test data. It is deliberately offline: CI does not fetch a moving upstream branch or regenerate expected screens from Tide.

| Source | Pinned revision | Imported expectations | License |
|---|---|---|---|
| [Alacritty](https://github.com/alacritty/alacritty/tree/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/tests/ref) | `94e7c8874e526b1e67b349d9ba30ddf81669119e` | All 45 reference recordings, sizes, configs and cell grids from the release commit recorded in `alacritty_terminal 0.26.0`'s `.cargo_vcs_info.json` | Apache-2.0; original LICENSE-APACHE and LICENSE-MIT retained in `alacritty/` |
| [libvterm](https://github.com/neovim/libvterm/tree/934bc2fbf21800ac3458a499df8820ca5fb45fd3/t) | `934bc2fbf21800ac3458a499df8820ca5fb45fd3` | All eight `90vttest_*.test` scripts: movement, scroll regions, embedded controls, leading zeroes, wrapping, tabs and origin modes | MIT, Copyright (c) 2008 Paul Evans; original LICENSE retained in `libvterm/` |

Alacritty project contributors own the Alacritty materials. The grid replay method in `../upstream_compat.rs` is adapted from its [reference harness](https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/tests/ref.rs), under Apache-2.0. libvterm's expected strings remain unchanged. Its Perl runner is not copied or evaluated; Tide has a small Rust interpreter for the selected scripts' declarative instructions. Unsupported instructions fail, rather than being skipped. The archived libvterm repository provides historical VT expectations, not evidence of support for every current terminal extension.

Alacritty recordings and JSON are losslessly gzip-compressed (about 47 MB of originals occupy about 300 KB). `manifest.json` records the repository, commit, original path, original byte length and SHA-256, plus the stored-file SHA-256. Decompression returns the exact upstream bytes. Terminal escapes in recordings are untrusted test input: the harness feeds the in-memory terminal directly and never prints them to an outer terminal, evaluates them as commands or launches recorded applications. The test's file paths and hostnames are public upstream fixture data, not captured Tide user sessions.

Run:

```bash
python3 scripts/upstream_corpus.py verify
cargo test --test upstream_compat --locked -- --test-threads=1
```

There are 53 named compatibility scenarios, each replayed with whole writes and chunks of 1, 7 and 4096 bytes (212 scenario replays). A further test checks file hashes and exact registration: every imported scenario must have a named Rust test. The existing GitHub Actions `cargo test --all-targets` step automatically runs these tests and includes their results in `rust-tests.log`. No additional CI downloads or system packages are required; `flate2` is a development-only dependency.

The Alacritty tier compares cells, attributes, wide/combining content, scrollback and grid dimensions/offset using the upstream normalized-storage equality. Older recorded `raw.visible_lines` values are not suitable for indexing; the harness follows upstream's storage comparison instead. Process-global, automatically generated hyperlink IDs are relabeled by first occurrence, preserving URIs and link identity relationships. Explicit IDs are retained. Cursor positions, modes and Tide's observer/snapshot output are compared across chunk sizes; Alacritty's saved grid format does not contain an independent cursor oracle. The libvterm tier retains its independent row and cursor expectations. Blank rows are padded to Tide's fixed screen width.

During validation, negative controls changed a reference grid cell, changed an independent expected row, corrupted a stored fixture and added an unknown script instruction. Each produced a nonzero failure; the original files were restored and all hashes reverified. These checks guard against a harness that merely replays input without enforcing the upstream oracle. They do not constitute a general mutation score.

Alacritty is Tide's underlying terminal core, so those recordings establish regression coverage, not an independent second implementation. The libvterm expectations supplement that coverage. Neither tier establishes glyph rendering, accessibility, keyboard latency, GPU compatibility, or full VT conformance. Existing known deviations stay tracked separately; these scenarios require no skips or new exceptions.

To reproduce an import, clone both repositories into scratch directories, check out the exact commits above, then run:

```bash
python3 scripts/upstream_corpus.py import --alacritty /scratch/alacritty --libvterm /scratch/libvterm
```

The importer refuses a different HEAD. Upgrades require reviewing new upstream expectations, changing the pins intentionally, updating test registration, and investigating every failure. Never replace expected grids with Tide's current output to make tests pass.

Kitty was cloned and its `kitty_tests/screen.py` and test base inspected at `b9e7070cc83ff975297dfbb2b5090696e2ee6323`. Its tests instantiate Kitty's own C/Python screen engine and are GPL-3.0; they are not copied into this corpus or reported as passing against Tide. Their layered parser/screen/application testing approach informs future work. Konsole's Qt-specific autotests likewise need adapters rather than being runnable as Tide tests unchanged.
