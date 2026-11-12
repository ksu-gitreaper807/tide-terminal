mod support;
use alacritty_terminal::{event::Event, term::TermMode};
use serde_json::Value;
use support::Replay;
use tide_terminal::terminal;

fn check(id: &str) {
    let cases: Vec<Value> = serde_json::from_str(include_str!("cases/terminal.json")).unwrap();
    let case = cases
        .iter()
        .find(|case| case["id"] == id)
        .expect("Unknown fixture");
    let bytes = case["stream"].as_str().unwrap().as_bytes();
    let known_gaps: Value = serde_json::from_str(include_str!("cases/known-gaps.json")).unwrap();
    // PTYs have no message boundaries. Check identical independent expectations at each stride.
    for stride in [bytes.len().max(1), 1, 3, 11] {
        let mut replay = Replay::new(4, 12);
        for chunk in bytes.chunks(stride) {
            replay.feed(chunk);
        }
        let mut expected: Vec<String> = serde_json::from_value(case["screen"].clone()).unwrap();
        expected.resize(4, String::new());
        let actual = replay.lines();
        if let Some(gap) = known_gaps.get(id) {
            // Keep the correct oracle. A changed or fixed dependency must force re-triage,
            // rather than silently adding/retaining an exception.
            assert_ne!(
                actual, expected,
                "{id}: known gap now passes; remove the exception"
            );
            let observed: Vec<String> =
                serde_json::from_value(gap["observed_screen"].clone()).unwrap();
            assert_eq!(
                actual, observed,
                "{id}: known gap changed; re-triage required"
            );
            assert!(!gap["reason"].as_str().unwrap().is_empty());
        } else {
            assert_eq!(actual, expected, "{id}, stride {stride}: screen");
        }
        let view = terminal::snapshot(&replay.terminal);
        let cursor: (i32, usize) = serde_json::from_value(case["cursor"].clone()).unwrap();
        assert_eq!(view.cursor, cursor, "{id}, stride {stride}: cursor");
        if let Some(modes) = case["modes"].as_object() {
            for (name, enabled) in modes {
                let mode = match name.as_str() {
                    "alt_screen" => TermMode::ALT_SCREEN,
                    "bracketed_paste" => TermMode::BRACKETED_PASTE,
                    "app_cursor" => TermMode::APP_CURSOR,
                    "show_cursor" => TermMode::SHOW_CURSOR,
                    _ => panic!("Unknown mode {name}"),
                };
                assert_eq!(
                    view.mode.contains(mode),
                    enabled.as_bool().unwrap(),
                    "{id}: {name}"
                );
            }
        }
        if let Some(cells) = case["cells"].as_array() {
            for expected in cells {
                let row = expected["row"].as_u64().unwrap() as usize;
                let col = expected["col"].as_u64().unwrap() as usize;
                let actual = view
                    .cells
                    .iter()
                    .find(|cell| cell.row == row && cell.col == col)
                    .unwrap();
                if let Some(fg) = expected.get("fg") {
                    assert_eq!(serde_json::json!(actual.fg), *fg, "{id}: fg");
                }
                if let Some(bg) = expected.get("bg") {
                    assert_eq!(serde_json::json!(actual.bg), *bg, "{id}: bg");
                }
                if let Some(underline) = expected.get("underline") {
                    assert_eq!(actual.underline, underline.as_bool().unwrap(), "{id}");
                }
                if let Some(strike) = expected.get("strike") {
                    assert_eq!(actual.strike, strike.as_bool().unwrap(), "{id}");
                }
            }
        }
        let events: Vec<_> = replay.events.try_iter().collect();
        if let Some(title) = case["title"].as_str() {
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event,Event::Title(s) if s==title)),
                "{id}: title"
            );
        }
        if let Some(expected) = case.get("replies") {
            let replies: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    Event::PtyWrite(s) => Some(s),
                    _ => None,
                })
                .collect();
            assert_eq!(serde_json::json!(replies), *expected, "{id}: replies");
        }
    }
}
macro_rules! cases { ($($name:ident),* $(,)?) => { $(#[test] fn $name() { check(stringify!($name)); })* }; }
cases! {
    ascii,
    carriage_return,
    linefeed_preserves_column,
    crlf,
    backspace,
    tab_stop,
    cup,
    cup_zero,
    cursor_up,
    cursor_down,
    cursor_forward,
    cursor_backward,
    cursor_next_line,
    cursor_previous_line,
    horizontal_absolute,
    vertical_absolute,
    cursor_clamp,
    save_restore_dec,
    erase_line_right,
    erase_line_left,
    erase_line_all,
    erase_display_all,
    erase_display_below,
    erase_display_above,
    delete_characters,
    insert_characters,
    erase_characters,
    insert_mode,
    insert_line,
    delete_line,
    scroll_up,
    scroll_down,
    scroll_region,
    reverse_index,
    wrap,
    autowrap_disabled,
    wide_cjk,
    combining_accent,
    emoji,
    braille_box,
    nerd_private_use,
    alt_restores_primary,
    alt_mode,
    bracketed_paste,
    application_cursor,
    cursor_hidden,
    sgr_red,
    sgr_truecolor,
    sgr_256,
    sgr_gray,
    sgr_background,
    sgr_inverse,
    sgr_hidden,
    sgr_underline_strike,
    sgr_reset,
    dec_line_graphics,
    osc_title_bel,
    osc_title_st,
    dsr_cursor,
    dsr_status,
    reset_terminal,
    cancel_incomplete_csi,
}
