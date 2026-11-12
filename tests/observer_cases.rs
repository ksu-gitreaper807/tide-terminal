mod support;
use serde_json::Value;
use tide_terminal::observer::{Observer, ProtocolGuard};

fn check(id: &str) {
    let cases: Vec<Value> = serde_json::from_str(include_str!("cases/observer.json")).unwrap();
    let case = cases
        .iter()
        .find(|case| case["id"] == id)
        .expect("Unknown fixture");
    let bytes = case["stream"].as_str().unwrap().as_bytes();
    // Every byte boundary includes boundaries inside UTF-8, OSC, base64, CSI and ST.
    for split in 0..=bytes.len() {
        let mut guard = ProtocolGuard::default();
        let mut parser = vte::Parser::new();
        let mut observer = Observer::default();
        parser.advance(&mut observer, &guard.filter(&bytes[..split]));
        parser.advance(&mut observer, &guard.filter(&bytes[split..]));
        if let Some(hint) = case.get("hint") {
            assert_eq!(
                serde_json::json!(observer.input_hint()),
                *hint,
                "{id}, split {split}: hint"
            );
        }
        if let Some(text) = case["text"].as_str() {
            assert_eq!(observer.text(), text, "{id}, split {split}: text");
        }
        if let Some(expected) = case.get("markers") {
            assert_eq!(
                serde_json::json!(support::markers(&observer.markers)),
                *expected,
                "{id}, split {split}: markers"
            );
        }
        if let Some(expected) = case["alternate"].as_bool() {
            assert_eq!(observer.alternate, expected, "{id}");
        }
    }
}
macro_rules! cases { ($($name:ident),* $(,)?) => { $(#[test] fn $name() { check(stringify!($name)); })* }; }
cases! {
    password_0,
    password_1,
    password_2,
    password_3,
    password_4,
    password_5,
    password_6,
    confirmation_0,
    confirmation_1,
    confirmation_2,
    confirmation_3,
    confirmation_4,
    confirmation_5,
    confirmation_6,
    confirmation_7,
    confirmation_8,
    confirmation_9,
    fingerprint_0,
    fingerprint_1,
    fingerprint_2,
    fingerprint_3,
    fingerprint_4,
    fingerprint_5,
    fingerprint_6,
    fingerprint_7,
    fingerprint_8,
    fingerprint_9,
    fingerprint_10,
    fingerprint_11,
    no_prompt_0,
    no_prompt_1,
    no_prompt_2,
    no_prompt_3,
    no_prompt_4,
    no_prompt_5,
    no_prompt_6,
    no_prompt_7,
    no_prompt_8,
    no_prompt_9,
    no_prompt_10,
    no_prompt_11,
    alt_suppresses_47,
    alt_restores_47,
    alt_suppresses_1047,
    alt_restores_1047,
    alt_suppresses_1049,
    alt_restores_1049,
    text_utf8,
    text_colors,
    text_cr,
    text_backspace,
    text_tab,
    text_erase_right,
    text_line_rewrite,
    text_absolute_column,
    text_relative_column,
    text_newline,
    text_screen_clear,
    text_osc_ignored,
    text_bell,
    text_clear_scrollback,
    marker_prompt,
    marker_start,
    marker_end_success,
    marker_end_failure,
    marker_end_negative,
    marker_osc133_url,
    marker_osc133_bad_url,
    marker_cwd_osc7,
    marker_cwd_utf8,
    marker_cwd_bad_escape,
    marker_cwd_truncated_escape,
    marker_cwd_bad_utf8,
    marker_non_file_url,
    marker_unknown_osc,
    marker_unknown_tide,
    marker_input_request,
    marker_bad_base64,
    marker_bad_base64_utf8,
    marker_command_bel,
    marker_command_st,
    marker_cwd_bel,
    marker_cwd_st,
}
