mod support;
use support::Replay;
use tide_terminal::terminal;
#[test]
fn ascii_range() {
    let mut replay = Replay::new(4, 20);
    replay.feed(b"abcdef");
    assert_eq!(terminal::selected(&replay.terminal, (0, 1), (0, 3)), "bcd");
}
#[test]
fn reverse_drag_uses_same_range() {
    let mut replay = Replay::new(4, 20);
    replay.feed(b"abcdef");
    assert_eq!(terminal::selected(&replay.terminal, (0, 3), (0, 1)), "bcd");
}
#[test]
fn column_past_edge_is_clamped() {
    let mut replay = Replay::new(4, 20);
    replay.feed(b"abcdef");
    assert_eq!(
        terminal::selected(&replay.terminal, (0, 0), (0, usize::MAX)),
        "abcdef"
    );
}
#[test]
fn wide_character_is_selected_once() {
    let mut replay = Replay::new(4, 20);
    replay.feed("日本".as_bytes());
    assert_eq!(terminal::selected(&replay.terminal, (0, 0), (0, 1)), "日");
}
#[test]
fn combining_mark_stays_with_base() {
    let mut replay = Replay::new(4, 20);
    replay.feed("e\u{301}x".as_bytes());
    assert_eq!(
        terminal::selected(&replay.terminal, (0, 0), (0, 0)),
        "e\u{301}"
    );
}
#[test]
fn selection_across_hard_newline() {
    let mut replay = Replay::new(4, 20);
    replay.feed(b"first\r\nsecond");
    assert_eq!(
        terminal::selected(&replay.terminal, (0, 0), (1, 5)),
        "first\nsecond"
    );
}
#[test]
fn soft_wrap_does_not_insert_a_newline() {
    let mut replay = Replay::new(4, 4);
    replay.feed(b"abcdef");
    assert_eq!(
        terminal::selected(&replay.terminal, (0, 0), (1, 1)),
        "abcdef"
    );
}
#[test]
fn scrolled_selection_uses_visible_rows() {
    let mut replay = Replay::new(4, 20);
    replay.feed(b"line0\r\nline1\r\nline2\r\nline3\r\nline4\r\nline5");
    terminal::scroll(&mut replay.terminal, 100);
    assert_eq!(
        terminal::selected(&replay.terminal, (0, 0), (0, 4)),
        "line0"
    );
}

fn history() -> Replay {
    let mut replay = Replay::new(4, 20);
    replay.feed(
        (0..10)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\r\n")
            .as_bytes(),
    );
    replay
}

#[test]
fn retained_selection_does_not_change_text_when_viewport_scrolls() {
    let mut r = history();
    terminal::begin_selection(&mut r.terminal, (1, 0));
    terminal::extend_selection(&mut r.terminal, (1, 4));
    assert_eq!(r.terminal.selection_to_string().as_deref(), Some("line7"));
    let range = terminal::snapshot(&r.terminal).selection;
    for delta in [2, 100, -1, -100] {
        terminal::scroll(&mut r.terminal, delta);
        assert_eq!(r.terminal.selection_to_string().as_deref(), Some("line7"));
        assert_eq!(terminal::snapshot(&r.terminal).selection, range);
    }
}

#[test]
fn backward_drag_expands_when_history_scrolls_under_stationary_pointer() {
    let mut r = history();
    terminal::begin_selection(&mut r.terminal, (1, 4));
    terminal::extend_selection(&mut r.terminal, (1, 0));
    terminal::scroll(&mut r.terminal, 2);
    terminal::extend_selection(&mut r.terminal, (1, 0));
    assert_eq!(
        r.terminal.selection_to_string().as_deref(),
        Some("line5\nline6\nline7")
    );
    terminal::scroll(&mut r.terminal, 2);
    terminal::extend_selection(&mut r.terminal, (1, 0));
    assert_eq!(
        r.terminal.selection_to_string().as_deref(),
        Some("line3\nline4\nline5\nline6\nline7")
    );
}

#[test]
fn forward_drag_can_select_more_than_a_screenful() {
    let mut r = history();
    terminal::scroll(&mut r.terminal, 100);
    terminal::begin_selection(&mut r.terminal, (1, 0));
    terminal::extend_selection(&mut r.terminal, (3, 4));
    terminal::scroll(&mut r.terminal, -3);
    terminal::extend_selection(&mut r.terminal, (3, 4));
    assert_eq!(
        r.terminal.selection_to_string().as_deref(),
        Some("line1\nline2\nline3\nline4\nline5\nline6")
    );
}

#[test]
fn retained_selection_follows_content_when_new_output_rotates_grid() {
    let mut r = history();
    terminal::begin_selection(&mut r.terminal, (0, 0));
    terminal::extend_selection(&mut r.terminal, (0, 4));
    r.feed(b"\r\nline10\r\nline11");
    assert_eq!(r.terminal.selection_to_string().as_deref(), Some("line6"));
    assert!(
        terminal::snapshot(&r.terminal)
            .selection
            .unwrap()
            .start
            .line
            .0
            < 0
    );
}

#[test]
fn retained_selection_preserves_wide_combining_and_soft_wrapped_text() {
    for (cols, text, end, expected) in [
        (20, "日本", (0, 1), "日"),
        (20, "e\u{301}x", (0, 0), "e\u{301}"),
        (4, "abcdef", (1, 1), "abcdef"),
    ] {
        let mut r = Replay::new(4, cols);
        r.feed(text.as_bytes());
        terminal::begin_selection(&mut r.terminal, end);
        terminal::extend_selection(&mut r.terminal, (0, 0));
        assert_eq!(r.terminal.selection_to_string().as_deref(), Some(expected));
    }
}
