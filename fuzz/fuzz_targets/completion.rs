#![no_main]
use libfuzzer_sys::fuzz_target;
use tide_terminal::split_capture::{Channel, SplitCapture};
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn status(data: &[u8]) -> Option<i32> {
    if data.is_empty() || data.iter().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    let number = std::str::from_utf8(data).ok()?.parse::<u64>().ok()?;
    (number <= 255).then_some(number as i32)
}
fuzz_target!(|data: &[u8]| {
    let payload = data.get(1..).unwrap_or_default();
    let split = data
        .first()
        .map_or(0, |v| usize::from(*v) % (payload.len() + 1));
    // Restrict payload bytes to prevent the harness's status field becoming
    // another escape sequence; metadata.rs independently fuzzes arbitrary bytes.
    let clean: Vec<u8> = payload
        .iter()
        .map(|b| match b {
            b'0'..=b'9' | b'+' | b'-' => *b,
            _ => b'x',
        })
        .collect();
    let (a, b) = clean.split_at(split);
    let expected = match (status(a), status(b)) {
        (Some(a), Some(b)) if a == b => a,
        _ => -1,
    };
    let mut capture = SplitCapture::new(None, "/tmp".into(), "/bin/bash".into(), true);
    capture.require_integration_token(Some(TOKEN.into()));
    let mut parsers = [vte::Parser::new(), vte::Parser::new()];
    for (channel, value) in [(Channel::Stdout, a), (Channel::Stderr, b)] {
        capture.select(channel);
        let bytes = format!(
            "\x1b]777;tide;v1;{TOKEN};start;\x07retained\x1b]777;tide;v1;{TOKEN};end;{}\x07",
            std::str::from_utf8(value).unwrap()
        );
        for part in bytes
            .as_bytes()
            .chunks(usize::from(data.first().copied().unwrap_or(0)) + 1)
        {
            parsers[channel as usize].advance(&mut capture, part);
        }
    }
    let completed = capture.completed.lock().unwrap();
    let completed = completed.as_ref().expect("both boundaries must complete");
    assert_eq!(completed.exit_code, Some(expected));
    assert!(completed.output.contains("retained"));
});
