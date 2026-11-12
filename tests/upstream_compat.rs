//! Offline upstream expectations through Tide's guard, parser and snapshot adapter.
//! Grid replay adapts Alacritty's Apache-2.0 ref.rs method; see upstream/README.md.
mod support;

use alacritty_terminal::{
    grid::{Dimensions, Grid},
    term::{Config, cell::Cell, test::TermSize},
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::PathBuf,
    process::Command,
};
use support::Replay;

const STRIDES: [usize; 4] = [usize::MAX, 1, 7, 4096];

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/upstream")
}

fn decompress(path: PathBuf) -> Vec<u8> {
    let mut bytes = Vec::new();
    GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[derive(Deserialize)]
struct RefConfig {
    history_size: usize,
}

fn alacritty_recording(name: &str) {
    let path = corpus()
        .join("alacritty/alacritty_terminal/tests/ref")
        .join(name);
    let recording = decompress(path.join("alacritty.recording.gz"));
    let explicit_link_ids: HashSet<String> = regex::bytes::Regex::new(r"\x1b\]8;([^;\x1b\x07]*);")
        .unwrap()
        .captures_iter(&recording)
        .flat_map(|capture| {
            String::from_utf8_lossy(&capture[1])
                .split(':')
                .filter_map(|param| param.strip_prefix("id=").map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect();
    let size: TermSize = serde_json::from_slice(&decompress(path.join("size.json.gz"))).unwrap();
    let expected: Grid<Cell> =
        serde_json::from_slice(&decompress(path.join("grid.json.gz"))).unwrap();
    let config: RefConfig =
        serde_json::from_slice(&decompress(path.join("config.json.gz"))).unwrap();
    let mut whole_snapshot = None;
    for stride in STRIDES {
        let mut replay = Replay::new(size.screen_lines(), size.columns());
        replay.terminal.set_options(Config {
            scrolling_history: config.history_size,
            ..Default::default()
        });
        for chunk in recording.chunks(stride) {
            replay.feed(chunk);
        }
        // Like the upstream harness, compare initialized, truncated storage including history,
        // wide/combining cells, colors and attributes. Cursor state is not serialized upstream;
        // Tide's snapshot and current cursor are additionally checked for chunk equivalence.
        let mut actual = replay.terminal.grid().clone();
        actual.initialize_all();
        actual.truncate();
        assert_eq!(
            actual.total_lines(),
            expected.total_lines(),
            "{name}/{stride}: lines"
        );
        assert_eq!(
            actual.columns(),
            expected.columns(),
            "{name}/{stride}: columns"
        );
        if actual != expected
            && canonical_links(&actual, &explicit_link_ids)
                != canonical_links(&expected, &explicit_link_ids)
        {
            // Older recordings have a stale raw.visible_lines field. Grid equality correctly
            // compares the normalized ring storage; indexing the old expected grid would not.
            let a = serde_json::to_value(&actual).unwrap();
            let e = serde_json::to_value(&expected).unwrap();
            for (row, (a, e)) in a["raw"]["inner"]
                .as_array()
                .unwrap()
                .iter()
                .zip(e["raw"]["inner"].as_array().unwrap())
                .enumerate()
            {
                for (col, (a, e)) in a["inner"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(e["inner"].as_array().unwrap())
                    .enumerate()
                {
                    assert_eq!(a, e, "{name}/{stride}: storage cell ({row}, {col})");
                }
            }
            panic!("{name}/{stride}: grid metadata differs");
        }
        let snapshot = replay.fingerprint();
        if let Some(whole) = &whole_snapshot {
            assert_eq!(
                &snapshot, whole,
                "{name}/{stride}: Tide snapshot/observer chunk divergence"
            );
        } else {
            whole_snapshot = Some(snapshot);
        }
    }
}

// Auto hyperlink IDs use a process-global counter. Canonicalize their labels, preserving
// URI, explicit IDs and the equality/distinctness of every link in the recording.
// NUL in canonical labels cannot collide with a valid OSC ID (ProtocolGuard cancels NUL).
fn canonical_links(grid: &Grid<Cell>, explicit: &HashSet<String>) -> Grid<Cell> {
    let mut value = serde_json::to_value(grid).unwrap();
    let mut labels = HashMap::new();
    for row in value["raw"]["inner"].as_array_mut().unwrap() {
        for cell in row["inner"].as_array_mut().unwrap() {
            if let Some(id) = cell.pointer_mut("/extra/hyperlink/inner/id")
                && let Some(text) = id.as_str()
                && let Some(prefix) = text.strip_suffix("_alacritty")
                && prefix.parse::<u32>().is_ok()
                && !explicit.contains(text)
            {
                let next = labels.len();
                let label = *labels.entry(text.to_owned()).or_insert(next);
                *id = serde_json::json!(format!("\0tide-test-auto-link-{label}"));
            }
        }
    }
    serde_json::from_value(value).unwrap()
}

macro_rules! recordings {
    ($($name:ident),+ $(,)?) => {
        const ALACRITTY_CASES: &[&str] = &[$(stringify!($name)),+];
        $(#[test]
        fn $name() { alacritty_recording(stringify!($name)); })+
    };
}

recordings!(
    alt_reset,
    clear_underline,
    colored_reset,
    colored_underline,
    csi_rep,
    decaln_reset,
    deccolm_reset,
    delete_chars_reset,
    delete_lines,
    erase_chars_reset,
    fish_cc,
    grid_reset,
    history,
    hyperlinks,
    indexed_256_colors,
    insert_blank_reset,
    issue_855,
    ll,
    newline_with_cursor_beyond_scroll_region,
    region_scroll_down,
    row_reset,
    saved_cursor,
    saved_cursor_alt,
    scroll_up_reset,
    selective_erasure,
    sgr,
    tab_rendering,
    tmux_git_log,
    tmux_htop,
    underline,
    vim_24bitcolors_bce,
    vim_large_window_scroll,
    vim_simple_edit,
    vttest_cursor_movement_1,
    vttest_insert,
    vttest_origin_mode_1,
    vttest_origin_mode_2,
    vttest_scroll,
    vttest_tab_clear_set,
    wrapline_alt_toggle,
    zerowidth,
    zsh_tab_completion,
    erase_in_line,
    scroll_in_region_up_preserves_history,
    origin_goto,
);

// Only the small declarative dialect used by the eight selected files is accepted.
// Unlike libvterm's Perl runner, nothing in a fixture is eval'd or executed by a shell.
fn unquote(text: &str) -> Vec<u8> {
    let mut input = text
        .strip_prefix('"')
        .unwrap()
        .strip_suffix('"')
        .unwrap()
        .bytes();
    let mut result = Vec::new();
    while let Some(byte) = input.next() {
        if byte != b'\\' {
            result.push(byte);
            continue;
        }
        result.push(match input.next().unwrap() {
            b'e' => 0x1b,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'b' => 8,
            b'\\' => b'\\',
            b'"' => b'"',
            b'x' => {
                let digits = [input.next().unwrap(), input.next().unwrap()];
                u8::from_str_radix(std::str::from_utf8(&digits).unwrap(), 16).unwrap()
            }
            escape => panic!("unsupported fixture escape: {escape}"),
        });
    }
    result
}

fn libvterm_line(replay: &mut Replay, line: &str, stride: usize, context: &str) {
    if let Some(sequence) = line.strip_prefix("$SEQ ") {
        let (bounds, command) = sequence.split_once(':').unwrap();
        let bounds: Vec<usize> = bounds
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        assert_eq!(bounds.len(), 2);
        for value in bounds[0]..=bounds[1] {
            libvterm_line(
                replay,
                command.trim().replace("\\#", &value.to_string()).as_str(),
                stride,
                context,
            );
        }
    } else if let Some(repeat) = line.strip_prefix("$REP ") {
        let (count, command) = repeat.split_once(':').unwrap();
        for _ in 0..count.trim().parse::<usize>().unwrap() {
            libvterm_line(replay, command.trim(), stride, context);
        }
    } else if let Some(text) = line.strip_prefix("PUSH ") {
        for chunk in unquote(text.trim()).chunks(stride) {
            replay.feed(chunk);
        }
    } else if let Some(assertion) = line.strip_prefix("?screen_row ") {
        let (row, text) = assertion.split_once('=').unwrap();
        let expected = String::from_utf8(unquote(text.trim())).unwrap();
        // libvterm returns variable-length rows; Tide returns fixed-width blank padding.
        let expected = format!("{expected:80}");
        let mut actual = vec![" ".to_owned(); 80];
        let row = row.trim().parse::<usize>().unwrap();
        let view = tide_terminal::terminal::snapshot(&replay.terminal);
        for cell in view.cells.iter().filter(|cell| cell.row == row) {
            actual[cell.col].clone_from(&cell.text);
        }
        assert_eq!(actual.concat(), expected, "{context}/{stride}: row {row}");
    } else if let Some(cursor) = line.strip_prefix("?cursor = ") {
        let (row, col) = cursor.split_once(',').unwrap();
        let view = tide_terminal::terminal::snapshot(&replay.terminal);
        assert_eq!(
            view.cursor,
            (row.parse().unwrap(), col.parse().unwrap()),
            "{context}/{stride}: cursor"
        );
    } else if line == "RESET" {
        *replay = Replay::new(24, 80);
    } else if !(line.is_empty()
        || line.starts_with('#')
        || line.starts_with('!')
        || matches!(line, "INIT" | "WANTSTATE" | "WANTSCREEN"))
    {
        panic!("{context}: unsupported fixture instruction: {line}");
    }
}

fn libvterm_script(name: &str) {
    let script =
        fs::read_to_string(corpus().join("libvterm/t").join(format!("{name}.test"))).unwrap();
    for stride in STRIDES {
        let mut replay = Replay::new(24, 80);
        for (number, line) in script.lines().enumerate() {
            libvterm_line(
                &mut replay,
                line.trim(),
                stride,
                &format!("{name}:{}", number + 1),
            );
        }
    }
}

macro_rules! scripts {
    ($($name:ident => $file:literal),+ $(,)?) => {
        const LIBVTERM_CASES: &[&str] = &[$($file),+];
        $(#[test]
        fn $name() { libvterm_script($file); })+
    };
}

scripts!(
    independent_movement_frame => "90vttest_01-movement-1",
    independent_scroll_region => "90vttest_01-movement-2",
    independent_embedded_controls => "90vttest_01-movement-3",
    independent_leading_zeroes => "90vttest_01-movement-4",
    independent_wrap_mode => "90vttest_02-screen-1",
    independent_tab_stops => "90vttest_02-screen-2",
    independent_origin_mode => "90vttest_02-screen-3",
    independent_absolute_mode => "90vttest_02-screen-4",
);

#[test]
fn upstream_corpus_integrity_and_registration() {
    let status = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/upstream_corpus.py"
        ))
        .arg("verify")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "upstream hashes, file registration or revision pins changed"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(corpus().join("manifest.json")).unwrap()).unwrap();
    for (project, names, root, extension) in [
        (
            "alacritty",
            ALACRITTY_CASES,
            "alacritty/alacritty_terminal/tests/ref",
            "",
        ),
        ("libvterm", LIBVTERM_CASES, "libvterm/t", ".test"),
    ] {
        assert_eq!(manifest["cases"][project], names.len());
        let mut registered: Vec<_> = names.iter().map(|s| format!("{s}{extension}")).collect();
        registered.sort();
        let mut actual: Vec<_> = fs::read_dir(corpus().join(root))
            .unwrap()
            .map(|p| p.unwrap().file_name().to_str().unwrap().to_owned())
            .collect();
        actual.sort();
        assert_eq!(
            actual, registered,
            "{project}: every fixture must have a named test"
        );
    }
}
