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
