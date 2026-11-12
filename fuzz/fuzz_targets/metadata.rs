#![no_main]
use libfuzzer_sys::fuzz_target;
use tide_terminal::observer::{Marker, Observer, ProtocolGuard};
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn replay(data: &[u8], stride: usize) -> Vec<Marker> {
    let mut guard = ProtocolGuard::default();
    let mut parser = vte::Parser::new();
    let mut observer = Observer::with_integration_token(Some(TOKEN.into()));
    for part in data.chunks(stride) {
        parser.advance(&mut observer, &guard.filter(part));
        assert!(observer.text().len() <= 131072);
    }
    observer.markers
}
fuzz_target!(|data: &[u8]| {
    let stride = data.first().map_or(1, |v| usize::from(*v) + 1);
    let markers = replay(data, stride);
    if !data
        .windows(TOKEN.len())
        .any(|part| part == TOKEN.as_bytes())
    {
        assert!(
            markers
                .iter()
                .all(|marker| matches!(marker, Marker::Input(_)))
        );
    }
    // The dependency has a documented malformed-UTF8 chunk deviation. Valid
    // UTF8 retains the stronger marker equivalence oracle; arbitrary bytes still
    // exercise safety and the authority assertion above.
    if std::str::from_utf8(data).is_ok() {
        assert_eq!(markers, replay(data, data.len().max(1)));
    }
});
