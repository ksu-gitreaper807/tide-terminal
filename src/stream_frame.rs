//! Prevent a PTY read boundary from splicing another channel into UTF-8 or ANSI.
#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Intermediate,
    Csi,
    String {
        osc: bool,
        escape: bool,
    },
    Utf8(u8),
}
#[derive(Default)]
pub(crate) struct Frames {
    pending: Vec<u8>,
    state: State,
}
impl Frames {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut boundary = 0;
        for &b in bytes {
            self.pending.push(b);
            self.state = match self.state {
                State::Ground => ground(b),
                State::Utf8(left) if b & 0xc0 == 0x80 => {
                    if left == 1 {
                        State::Ground
                    } else {
                        State::Utf8(left - 1)
                    }
                }
                State::Utf8(_) => ground(b),
                _ if matches!(b, 0x18 | 0x1a) => State::Ground,
                State::Escape => match b {
                    b'[' => State::Csi,
                    b']' | b'P' | b'^' | b'_' | b'X' => State::String {
                        osc: b == b']',
                        escape: false,
                    },
                    0x20..=0x2f => State::Intermediate,
                    0x1b => State::Escape,
                    _ => State::Ground,
                },
                State::Intermediate => {
                    if b == 0x1b {
                        State::Escape
                    } else if (0x30..=0x7e).contains(&b) {
                        State::Ground
                    } else {
                        State::Intermediate
                    }
                }
                State::Csi => {
                    if b == 0x1b {
                        State::Escape
                    } else if (0x40..=0x7e).contains(&b) {
                        State::Ground
                    } else {
                        State::Csi
                    }
                }
                State::String { osc, escape } => {
                    if (osc && b == 7) || (escape && b == b'\\') {
                        State::Ground
                    } else {
                        State::String {
                            osc,
                            escape: b == 0x1b,
                        }
                    }
                }
            };
            if matches!(self.state, State::Ground) {
                boundary = self.pending.len();
            }
            if self.pending.len() - boundary > 65_536 {
                self.pending.push(0x18);
                self.state = State::Ground;
                boundary = self.pending.len();
            }
        }
        self.pending.drain(..boundary).collect()
    }
}
fn ground(b: u8) -> State {
    match b {
        0x1b => State::Escape,
        0xc2..=0xdf => State::Utf8(1),
        0xe0..=0xef => State::Utf8(2),
        0xf0..=0xf4 => State::Utf8(3),
        _ => State::Ground,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holds_only_the_incomplete_utf8_suffix() {
        let mut f = Frames::default();
        assert_eq!(f.filter(b"A\xe6\x97"), b"A");
        assert_eq!(f.filter(b"\xa5B"), "日B".as_bytes());
    }
    #[test]
    fn ansi_sequences_are_never_split_between_channels() {
        for sequence in [
            b"\x1b[31m".as_slice(),
            b"\x1b]133;D;0\x07",
            b"\x1b]0;title\x1b\\",
            b"\x1bPpayload\x1b\\",
            b"\x1b(B",
        ] {
            for split in 1..sequence.len() {
                let mut f = Frames::default();
                assert!(f.filter(&sequence[..split]).is_empty());
                assert_eq!(f.filter(&sequence[split..]), sequence);
            }
        }
    }
    #[test]
    fn cancel_and_oversized_strings_recover() {
        let mut f = Frames::default();
        assert!(f.filter(b"\x1b]unfinished").is_empty());
        assert_eq!(f.filter(b"\x18ok"), b"\x1b]unfinished\x18ok");
        let _ = f.filter(b"\x1bX");
        let result = f.filter(&vec![b'x'; 70_000]);
        assert!(result.contains(&0x18));
        assert!(f.pending.len() <= 65_536);
        assert_eq!(f.filter(b"OK"), b"OK");
    }
    #[test]
    fn ordinary_text_and_controls_are_not_delayed() {
        let mut f = Frames::default();
        assert_eq!(f.filter(b"a\r\nb\t\x03"), b"a\r\nb\t\x03");
    }
}
