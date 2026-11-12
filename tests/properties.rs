mod support;
use proptest::prelude::*;
use std::time::Duration;
use support::Replay;
use tide_terminal::{
    observer::{Observer, ProtocolGuard},
    rules::{RuleSpec, Rules},
    terminal::{self, Size},
};

fn chunked(bytes: &[u8], widths: &[usize], rows: usize, cols: usize) -> Replay {
    let mut replay = Replay::new(rows, cols);
    let mut offset = 0;
    for width in widths.iter().copied().cycle() {
        if offset == bytes.len() {
            break;
        }
        let end = (offset + width).min(bytes.len());
        replay.feed(&bytes[offset..end]);
        offset = end;
    }
    replay
}
fn fragments() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 ]{0,20}",
        Just("日本 e\u{301}🙂⣿\u{f07b}".into()),
        Just("\r\n\t\x08".into()),
        Just("\x1b[31mred\x1b[0m".into()),
        Just("\x1b[2J\x1b[H".into()),
        Just("\x1b[2;3H\x1b[2K".into()),
        Just("\x1b[?1049hEDITOR\x1b[?1049l".into()),
        Just("\x1b]2;fixture\x1b\\".into()),
        Just("\x1b]133;C\x07\x1b]133;D;7\x07".into()),
        Just("\x1b]777;tide;command;ZWNobyBoaQ==\x07".into()),
        Just("\x1b[?2004h\x1b[?2004l".into()),
    ]
}
proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters:4096,failure_persistence:Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct("tests/properties.proptest-regressions"))),..ProptestConfig::default() })]
    #[test]
    fn structured_terminal_stream_is_independent_of_pty_chunk_boundaries(
        parts in prop::collection::vec(fragments(),0..30), widths in prop::collection::vec(1usize..64,1..20)
    ) {
        let stream=parts.concat();
        let mut whole=Replay::new(8,40);
        whole.feed(stream.as_bytes());
        let split=chunked(stream.as_bytes(),&widths,8,40);
        prop_assert_eq!(whole.fingerprint(),split.fingerprint());
    }
    #[test]
    fn arbitrary_bytes_remain_bounded_and_recover_after_reset(
        bytes in prop::collection::vec(any::<u8>(),0..2048), widths in prop::collection::vec(1usize..64,1..20)
    ) {
        // Error recovery for malformed UTF-8 is implementation-dependent. Assert safety
        // and reset recovery here; valid structured streams have a stronger equality oracle.
        let mut whole=Replay::new(8,40);
        whole.feed(&bytes);
        let mut split=chunked(&bytes,&widths,8,40);
        for replay in [&mut whole,&mut split] {
            let view=terminal::snapshot(&replay.terminal);
            prop_assert!(view.cells.iter().all(|cell|cell.row<8 && cell.col<40));
            replay.feed(b"\x18\x18\x1bc\x1b[2J\x1b[HRECOVERY");
            let lines=replay.lines();
            prop_assert_eq!(lines[0].as_str(),"RECOVERY");
            prop_assert!(replay.observer.tail().ends_with("RECOVERY"));
        }
    }
    #[test]
    fn guard_preserves_filter_state_across_chunks(
        bytes in prop::collection::vec(any::<u8>(),0..4096), width in 1usize..128
    ) {
        let expected=ProtocolGuard::default().filter(&bytes);
        let mut guard=ProtocolGuard::default();
        let actual:Vec<u8>=bytes.chunks(width).flat_map(|part|guard.filter(part)).collect();
        prop_assert_eq!(actual,expected);
    }
    #[test]
    fn resize_keeps_cells_and_cursor_inside_grid(
        parts in prop::collection::vec(fragments(),0..20), sizes in prop::collection::vec((2usize..40,2usize..120),1..20)
    ) {
        let mut replay=Replay::new(8,40);
        replay.feed(parts.concat().as_bytes());
        for (rows,cols) in sizes {
            replay.terminal.resize(Size{rows,cols});
            let view=terminal::snapshot(&replay.terminal);
            prop_assert_eq!((view.rows,view.cols),(rows,cols));
            prop_assert!(view.cursor.0>=0 && (view.cursor.0 as usize)<rows && view.cursor.1<cols);
            prop_assert!(view.cells.iter().all(|c|c.row<rows && c.col<cols));
        }
    }
    #[test]
    fn runtime_boundary_and_once_per_run(limit in 1u64..1_000_000,extra in 0u64..1_000_000) {
        let mut rules=Rules::new(RuleSpec{timeout:Some(Duration::from_nanos(limit)),..Default::default()}).unwrap();
        prop_assert!(rules.evaluate(Duration::from_nanos(limit-1),Duration::ZERO,"").is_none());
        prop_assert!(rules.evaluate(Duration::from_nanos(limit),Duration::ZERO,"").is_some());
        prop_assert!(rules.evaluate(Duration::from_nanos(limit+extra),Duration::ZERO,"").is_none());
    }
    #[test]
    fn literal_match_treats_regex_metacharacters_literally(marker in "[a-zA-Z0-9.\\[\\]()*+?^$|]{1,40}") {
        let mut rules=Rules::new(RuleSpec{literal:Some(marker.clone()),..Default::default()}).unwrap();
        let output=format!("prefix\n{marker}\nsuffix");
        prop_assert!(rules.evaluate(Duration::ZERO,Duration::ZERO,&output).is_some());
    }
    #[test]
    fn unicode_observer_line_stays_bounded(c in prop_oneof![Just('界'),Just('🙂'),Just('x')],length in 16385usize..50000) {
        let text:String=std::iter::repeat_n(c,length).collect();
        let mut observer=Observer::default();
        vte::Parser::new().advance(&mut observer,text.as_bytes());
        prop_assert!(observer.text().chars().count()<=16384);
        prop_assert!(observer.text().len()<=65536);
    }
}

#[test]
fn oversized_terminal_strings_are_cancelled_and_recover_for_all_introducers() {
    for start in *b"]P^_" {
        for ending in [b"\x1b\\".as_slice(), b"\x18", b"\x1a"] {
            for width in [1, 17, 8192] {
                let mut data = vec![27, start];
                data.extend(std::iter::repeat_n(b'x', 100_000));
                data.extend(ending);
                data.extend(b"RECOVERED");
                let mut guard = ProtocolGuard::default();
                let filtered: Vec<u8> = data
                    .chunks(width)
                    .flat_map(|part| guard.filter(part))
                    .collect();
                assert!(filtered.len() < 65_560, "{start}, {width}");
                assert!(filtered.contains(&0x18));
                let mut replay = Replay::new(4, 20);
                replay.feed(&filtered);
                assert_eq!(replay.observer.tail(), "RECOVERED", "{start}, {width}");
            }
        }
    }
}

#[test]
fn corpus_ids_are_unique_and_each_case_is_registered_as_a_test() {
    for (data, source) in [
        (
            include_str!("cases/terminal.json"),
            include_str!("terminal_conformance.rs"),
        ),
        (
            include_str!("cases/observer.json"),
            include_str!("observer_cases.rs"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(data).unwrap();
        let mut seen = std::collections::HashSet::new();
        for case in cases {
            let id = case["id"].as_str().unwrap();
            assert!(seen.insert(id.to_owned()), "duplicate fixture {id}");
            assert!(
                source.lines().any(|line| line.trim() == format!("{id},")),
                "Unregistered fixture {id}"
            );
        }
    }
}

#[test]
fn malformed_utf8_read_boundary_gap_is_tracked_and_resettable() {
    let gaps: serde_json::Value =
        serde_json::from_str(include_str!("cases/stream-known-gaps.json")).unwrap();
    let gap = &gaps["malformed_utf8_read_boundary"];
    let chunks: Vec<Vec<u8>> = serde_json::from_value(gap["chunks"].clone()).unwrap();
    let mut whole = Replay::new(4, 12);
    whole.feed(&chunks.concat());
    let mut split = Replay::new(4, 12);
    for chunk in chunks {
        split.feed(&chunk);
    }
    assert_ne!(
        whole.lines(),
        split.lines(),
        "Known gap now passes: remove or revise the exception"
    );
    assert_eq!(serde_json::json!(whole.lines()), gap["whole_screen"]);
    assert_eq!(serde_json::json!(split.lines()), gap["fragmented_screen"]);
    let recovery = gap["recovery_sequence"].as_str().unwrap().as_bytes();
    whole.feed(recovery);
    split.feed(recovery);
    assert_eq!(
        whole.fingerprint(),
        split.fingerprint(),
        "Malformed decoder state failed to recover"
    );
}
