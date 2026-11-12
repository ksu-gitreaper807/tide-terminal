use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::VecDeque;
use vte::{Params, Perform};

const LIMIT: usize = 16_384;

#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    Prompt,
    Start,
    End(i32),
    Command(String),
    Cwd(String),
    Input(String),
}

#[derive(Default)]
pub struct Observer {
    text_limit: Option<usize>,
    discarded: bool,
    line: Vec<char>,
    column: usize,
    lines: VecDeque<String>,
    retained: usize,
    pub markers: Vec<Marker>,
    pub alternate: bool,
    pub visible_activity: bool,
    pub bell: bool,
}

impl Observer {
    pub fn with_text_limit(limit: usize) -> Self {
        Self {
            text_limit: Some(limit.max(LIMIT)),
            ..Self::default()
        }
    }
    pub fn truncated(&self) -> bool {
        self.discarded
    }
    fn limit(&self) -> usize {
        self.text_limit.unwrap_or(LIMIT)
    }
    pub fn tail(&self) -> String {
        self.line.iter().collect::<String>().trim_end().to_owned()
    }
    pub fn text(&self) -> String {
        let mut result = String::with_capacity(self.retained + self.line.len());
        for line in &self.lines {
            result.push_str(line);
            result.push('\n');
        }
        result.extend(self.line.iter());
        result
    }
    pub fn clear_text(&mut self) {
        self.lines.clear();
        self.line.clear();
        self.column = 0;
        self.retained = 0;
        self.discarded = false;
    }
    fn newline(&mut self) {
        let line: String = self.line.iter().collect();
        self.retained += line.len() + 1;
        self.lines.push_back(line);
        while self.retained > self.limit() {
            if let Some(line) = self.lines.pop_front() {
                self.retained -= line.len() + 1;
                self.discarded = true;
            } else {
                break;
            }
        }
        self.line.clear();
        self.column = 0;
    }
    pub fn input_hint(&self) -> Option<&'static str> {
        if self.alternate {
            return None;
        }
        let tail = self.tail().to_lowercase();
        // PAM fingerprint instructions are often complete, newline-terminated
        // messages: the reader, rather than keyboard input, completes the scan.
        // Only fingerprint instructions use this completed-line fallback.
        let completed = if tail.is_empty() {
            self.lines
                .iter()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_lowercase())
        } else {
            None
        };
        if fingerprint_request(completed.as_deref().unwrap_or(&tail)) {
            Some("May need a fingerprint scan")
        } else if tail.contains("password:")
            || tail.contains("passphrase")
            || tail.contains("password for ")
            || tail.ends_with("pin:")
        {
            Some("May need a password")
        } else if [
            "[y/n]",
            "[y/n]:",
            "(y/n)",
            "[yes/no]",
            "(yes/no",
            "yes/no/",
            "press enter",
            "press return",
            "continue?",
            "proceed?",
        ]
        .iter()
        .any(|p| tail.contains(p))
        {
            Some("May need confirmation")
        } else {
            None
        }
    }
}

fn fingerprint_request(line: &str) -> bool {
    let line = line.trim();
    let line = line.strip_prefix("please ").unwrap_or(line);
    ["place your ", "swipe your "].iter().any(|prefix| {
        line.strip_prefix(prefix).is_some_and(|instruction| {
            ["finger on ", "thumb on ", "finger across ", "thumb across "]
                .iter()
                .any(|part| instruction.contains(part))
        })
    }) || [
        "scan your fingerprint",
        "scan your finger",
        "touch the fingerprint reader",
        "touch the fingerprint sensor",
        "press your finger on ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

impl Perform for Observer {
    fn print(&mut self, c: char) {
        self.visible_activity = true;
        if self.column >= self.text_limit.map(|limit| limit / 4).unwrap_or(LIMIT) {
            let trim = 2048.min(self.line.len());
            self.line.drain(..trim);
            self.column = self.column.saturating_sub(trim);
            self.discarded = true;
        }
        if self.column == self.line.len() {
            self.line.push(c);
        } else if self.column < self.line.len() {
            self.line[self.column] = c;
        } else {
            self.line.resize(self.column, ' ');
            self.line.push(c);
        }
        self.column += 1;
    }
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.visible_activity = true;
                self.newline();
            }
            b'\r' => self.column = 0,
            8 => self.column = self.column.saturating_sub(1),
            9 => {
                let stop = (self.column / 8 + 1) * 8;
                while self.column < stop && self.column < LIMIT {
                    self.print(' ');
                }
            }
            7 => self.bell = true,
            _ => {}
        }
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], _: bool) {
        let get = |i: usize| {
            params
                .get(i)
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("")
        };
        match get(0) {
            "133" => match get(1) {
                "A" => self.markers.push(Marker::Prompt),
                "C" => {
                    for param in params.iter().skip(2) {
                        if let Ok(s) = std::str::from_utf8(param)
                            && let Some(encoded) = s.strip_prefix("cmdline_url=")
                            && let Some(command) = percent_decode(encoded)
                        {
                            self.markers.push(Marker::Command(command));
                        }
                    }
                    self.markers.push(Marker::Start);
                }
                "D" => self.markers.push(Marker::End(get(2).parse().unwrap_or(0))),
                _ => {}
            },
            "777" if get(1) == "tide" => match get(2) {
                "command" if get(3).len() <= LIMIT => {
                    if let Ok(bytes) = STANDARD.decode(get(3))
                        && let Ok(s) = String::from_utf8(bytes)
                    {
                        self.markers.push(Marker::Command(s));
                    }
                }
                "cwd" if get(3).len() <= LIMIT => {
                    if let Ok(bytes) = STANDARD.decode(get(3))
                        && let Ok(s) = String::from_utf8(bytes)
                    {
                        self.markers.push(Marker::Cwd(s));
                    }
                }
                "input" => self.markers.push(Marker::Input("Input requested".into())),
                _ => {}
            },
            "7" => {
                if let Some((_, path)) = get(1)
                    .strip_prefix("file://")
                    .and_then(|s| s.split_once('/'))
                    && let Some(path) = percent_decode(&format!("/{path}"))
                {
                    self.markers.push(Marker::Cwd(path));
                }
            }
            _ => {}
        }
    }
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _: bool, action: char) {
        let p = params
            .iter()
            .next()
            .and_then(|p| p.first())
            .copied()
            .unwrap_or(0);
        if intermediates == b"?"
            && params
                .iter()
                .any(|p| matches!(p.first(), Some(47 | 1047 | 1049)))
            && matches!(action, 'h' | 'l')
        {
            self.alternate = action == 'h';
            self.clear_text();
        }
        match action {
            'K' => match p {
                0 => self.line.truncate(self.column),
                2 => {
                    self.line.clear();
                    self.column = 0;
                }
                _ => {}
            },
            'G' => self.column = usize::from(p.max(1) - 1).min(LIMIT),
            'C' => self.column = self.column.saturating_add(usize::from(p.max(1))).min(LIMIT),
            'D' => self.column = self.column.saturating_sub(usize::from(p.max(1))),
            'J' if p == 2 || p == 3 => self.clear_text(),
            _ => {}
        }
    }
}

fn percent_decode(s: &str) -> Option<String> {
    if s.len() > LIMIT {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len());
    let mut iter = s.as_bytes().iter().copied();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hi = (iter.next()? as char).to_digit(16)?;
            let lo = (iter.next()? as char).to_digit(16)?;
            bytes.push((hi * 16 + lo) as u8);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).ok()
}

/// Cap unfinished terminal strings before handing them to parsers with growable OSC buffers.
#[derive(Default)]
pub struct ProtocolGuard {
    escape: bool,
    in_string: bool,
    osc: bool,
    string_escape: bool,
    count: usize,
    discard: bool,
}
impl ProtocolGuard {
    pub fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        for &b in input {
            if self.in_string {
                let end = (self.osc && b == 7)
                    || (self.string_escape && b == b'\\')
                    || matches!(b, 0x18 | 0x1a);
                self.count += 1;
                if !self.discard && self.count > 65_536 {
                    output.push(0x18);
                    self.discard = true;
                }
                if !self.discard {
                    output.push(b);
                }
                self.string_escape = b == 0x1b;
                if end {
                    self.in_string = false;
                    self.escape = false;
                    self.discard = false;
                    self.string_escape = false;
                }
            } else {
                output.push(b);
                if self.escape && matches!(b, b']' | b'P' | b'^' | b'_') {
                    self.in_string = true;
                    self.osc = b == b']';
                    self.count = 0;
                    self.discard = false;
                    self.string_escape = false;
                }
                self.escape = b == 0x1b;
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feed(o: &mut Observer, p: &mut vte::Parser, bytes: &[u8]) {
        p.advance(o, bytes);
    }
    #[test]
    fn chunked_utf8_color_and_partial_prompts() {
        let mut o = Observer::default();
        let mut p = vte::Parser::new();
        for b in "\x1b[31m日本語\x1b[0m\nPassword:".as_bytes() {
            feed(&mut o, &mut p, &[*b]);
        }
        assert_eq!(o.input_hint(), Some("May need a password"));
        assert!(o.text().contains("日本語\nPassword:"));
        assert!(!o.text().contains("[31m"));
    }

    #[test]
    fn fingerprint_instructions_survive_newlines_without_matching_status_logs() {
        for prompt in [
            "Place your finger on the fingerprint reader",
            "Place your right index finger on the fingerprint reader\r\n",
            "Place your left thumb on Goodix\n\n",
            "Swipe your finger across the fingerprint reader\r\n",
            "Swipe your right thumb across the reader\n",
            "Please touch the fingerprint sensor",
            "Scan your fingerprint:",
            "\x1b[33mPlace your finger on the fingerprint reader\x1b[0m\r\n",
        ] {
            let mut observer = Observer::default();
            let mut parser = vte::Parser::new();
            for byte in prompt.as_bytes() {
                feed(&mut observer, &mut parser, &[*byte]);
            }
            assert_eq!(
                observer.input_hint(),
                Some("May need a fingerprint scan"),
                "{prompt:?}"
            );
            feed(
                &mut observer,
                &mut parser,
                b"\r\nAuthentication complete\r\n",
            );
            assert_eq!(observer.input_hint(), None);
            feed(&mut observer, &mut parser, b"Password:");
            assert_eq!(observer.input_hint(), Some("May need a password"));
        }
        for status in [
            "Fingerprint authentication successful\n",
            "Fingerprint reader not found\n",
            "ED25519 key fingerprint is SHA256:example\n",
            "Scanning file fingerprints...\n",
            "Place your fingers on the keyboard\n",
            "\x1b[?1049hPlace your finger on the fingerprint reader\n",
        ] {
            let mut observer = Observer::default();
            let mut parser = vte::Parser::new();
            feed(&mut observer, &mut parser, status.as_bytes());
            assert_eq!(observer.input_hint(), None, "{status:?}");
        }
    }
    #[test]
    fn progress_rewrite_and_editor_suppression() {
        let mut o = Observer::default();
        let mut p = vte::Parser::new();
        feed(&mut o, &mut p, b"Password:\rWorking..\x1b[K");
        assert!(o.input_hint().is_none());
        feed(&mut o, &mut p, b"\x1b[?1049hPassword:");
        assert!(o.input_hint().is_none());
        feed(&mut o, &mut p, b"\x1b[?1049lContinue? [y/N]");
        assert_eq!(o.input_hint(), Some("May need confirmation"));
    }
    #[test]
    fn markers_survive_split_sequences_and_output_is_bounded() {
        let mut o = Observer::default();
        let mut p = vte::Parser::new();
        for b in b"\x1b]133;C\x1b\\\x1b]777;tide;command;ZWNobyBoaQ==\x07" {
            feed(&mut o, &mut p, &[*b]);
        }
        assert_eq!(
            o.markers,
            vec![Marker::Start, Marker::Command("echo hi".into())]
        );
        feed(&mut o, &mut p, &vec![b'x'; 100_000]);
        assert!(o.text().len() <= LIMIT);
    }
    #[test]
    fn excessive_unfinished_osc_is_cancelled_and_next_output_survives() {
        let mut g = ProtocolGuard::default();
        let mut p = vte::Parser::new();
        let mut o = Observer::default();
        p.advance(&mut o, &g.filter(b"\x1b]0;"));
        for _ in 0..100 {
            p.advance(&mut o, &g.filter(&vec![b'x'; 8192]));
        }
        p.advance(&mut o, &g.filter(b"\x1b\\SURVIVED"));
        assert_eq!(o.tail(), "SURVIVED");
    }
}
