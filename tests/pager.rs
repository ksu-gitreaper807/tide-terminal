use std::time::{Duration, Instant};
use tide_terminal::session::{Event, Launch, Options, Session};

fn wait(s: &Session, mut predicate: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut events = vec![];
    while Instant::now() < deadline {
        if let Ok(event) = s.events.recv_timeout(Duration::from_millis(20)) {
            let done = predicate(&event);
            events.push(event);
            if done {
                return events;
            }
        }
    }
    panic!("pager event missing: {events:?}");
}

fn wait_output(s: &Session, needle: &str) {
    let mut received = String::new();
    wait(s, |event| {
        if let Event::Output { text } = event {
            received.push_str(text);
        }
        received.contains(needle)
    });
}

fn screen(s: &Session) -> String {
    s.snapshot()
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect()
}
fn wait_screen(s: &Session, predicate: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let text = screen(s);
        if predicate(&text) {
            return text;
        }
        assert!(Instant::now() < deadline, "pager screen missing: {text}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn actual_less_accepts_page_keys_and_quit_with_split_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages");
    std::fs::write(
        &path,
        (1..=200)
            .map(|i| format!("PAGER_LINE_{i:04}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let mut o = Options::shell("/bin/bash", dir.path().into());
    o.private = true;
    o.env = vec![("LESS".into(), "".into()), ("LESSOPEN".into(), "".into())];
    o.launch = Launch::Command(vec![
        "less".into(),
        "-R".into(),
        path.to_string_lossy().into(),
    ]);
    let s = Session::spawn(o).unwrap();
    wait_screen(&s, |text| text.contains("PAGER_LINE_0001"));
    s.write(b"\x1b[6~".to_vec()).unwrap();
    let next = wait_screen(&s, |text| {
        !text.contains("PAGER_LINE_0001") && text.contains("PAGER_LINE_")
    });
    assert!(
        !next.contains("^[]10;rgb:") && !next.contains("^[[?6c"),
        "{next}"
    );
    let first_line = |text: &str| {
        text.split("PAGER_LINE_")
            .nth(1)
            .and_then(|line| line.get(..4))
            .and_then(|line| line.parse::<u32>().ok())
    };
    let first = first_line(&next).unwrap();
    // xterm smkx enables application-cursor mode: use the corresponding
    // down-arrow encoding, as a terminal must also do for wheel emulation.
    assert!(
        s.snapshot()
            .mode
            .contains(alacritty_terminal::term::TermMode::APP_CURSOR)
    );
    s.write(b"\x1bOB".to_vec()).unwrap();
    wait_screen(&s, |text| first_line(text) == Some(first + 1));
    s.write(b"q".to_vec()).unwrap();
    let events = wait(&s, |e| matches!(e, Event::Exited { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Exited { code: 0, .. }))
    );
}

#[test]
fn man_pages_scroll_quit_and_restore_each_shells_keyboard() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = tempfile::tempdir().unwrap();
        let mut o = Options::shell(shell, dir.path().into());
        o.private = true;
        o.suggestions = false;
        o.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        o.env = vec![
            ("MANPAGER".into(), "less -R".into()),
            ("LESS".into(), "".into()),
            ("LESSOPEN".into(), "".into()),
        ];
        let s = Session::spawn(o).unwrap();
        wait(&s, |e| matches!(e, Event::Prompt));
        s.write(b"man man\n".to_vec()).unwrap();
        let first = wait_screen(&s, |text| text.contains("MAN(1)"));
        assert!(
            !first.contains("^[]10;rgb:") && !first.contains("^[[?6c"),
            "{shell}: {first}"
        );
        s.write(b" ".to_vec()).unwrap();
        wait_screen(&s, |text| text != first && !text.contains("MAN(1)"));
        s.write(b"q".to_vec()).unwrap();
        let events = wait(&s, |e| matches!(e, Event::CommandEnded { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CommandEnded { code: 0, .. })),
            "{shell}: {events:?}"
        );
        s.write(b"printf 'PAGER_SHELL_RETURNED\\n'\n".to_vec())
            .unwrap();
        let events = wait(&s, |e| matches!(e, Event::CommandEnded { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CommandEnded { code: 0, .. })),
            "{shell}: {events:?}"
        );
        assert!(screen(&s).contains("PAGER_SHELL_RETURNED"));
    }
}

#[test]
fn interrupt_still_reaches_foreground_group_when_pager_reads_secondary() {
    let dir = tempfile::tempdir().unwrap();
    let script = r#"
import os, select, signal, termios
fd = os.open(os.ttyname(2),os.O_RDONLY)
old = termios.tcgetattr(fd)
raw = termios.tcgetattr(fd)
raw[3] = (raw[3] & ~(termios.ICANON|termios.ECHO)) | termios.ISIG
raw[6][termios.VMIN] = 1
raw[6][termios.VTIME] = 0
termios.tcsetattr(fd,termios.TCSANOW,raw)
signal.signal(signal.SIGINT, lambda *_: os.write(1,b'PAGER_INTERRUPT_RECEIVED'))
os.write(1,b'PAGER_SIGNAL_READY')
assert select.select([fd],[],[],5)[0], 'NO_QUIT'
assert os.read(fd,1) == b'q'
termios.tcsetattr(fd,termios.TCSANOW,old)
"#;
    let mut o = Options::shell("/bin/bash", dir.path().into());
    o.private = true;
    o.launch = Launch::Command(vec![
        "python3".into(),
        "-u".into(),
        "-c".into(),
        script.into(),
    ]);
    let s = Session::spawn(o).unwrap();
    wait_output(&s, "PAGER_SIGNAL_READY");
    s.write(vec![3]).unwrap();
    wait_output(&s, "PAGER_INTERRUPT_RECEIVED");
    s.write(b"q".to_vec()).unwrap();
    let events = wait(&s, |e| matches!(e, Event::Exited { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Exited { code: 0, .. }))
    );
}

#[test]
fn terminal_replies_and_keyboard_reach_a_pager_reading_stderr_tty() {
    let dir = tempfile::tempdir().unwrap();
    let script = r#"
import os, select, termios, tty
fd = os.open(os.ttyname(2), os.O_RDONLY)
old = termios.tcgetattr(fd)
tty.setraw(fd)
os.write(1,b'\x1b]10;?\x07\x1b]11;?\x07\x1b[c')
expected = b'\x1b]10;rgb:d5d5/dcdc/e5e5\x07\x1b]11;rgb:0f0f/1414/1c1c\x07\x1b[?6c'
reply = b''
while len(reply) < len(expected):
    assert select.select([fd],[],[],2)[0], ('NO_REPLY',reply)
    reply += os.read(fd,4096)
assert reply == expected, reply
os.write(1,b'PAGER_KEYBOARD_READY')
assert select.select([fd],[],[],2)[0], 'NO_KEYBOARD'
assert os.read(fd,4096) == b'\x1b[6~q'
termios.tcsetattr(fd, termios.TCSANOW, old)
os.write(1,b'PAGER_OK')
"#;
    let mut o = Options::shell("/bin/bash", dir.path().into());
    o.private = true;
    o.launch = Launch::Command(vec![
        "python3".into(),
        "-u".into(),
        "-c".into(),
        script.into(),
    ]);
    let s = Session::spawn(o).unwrap();
    wait_output(&s, "PAGER_KEYBOARD_READY");
    s.write(b"\x1b[6~q".to_vec()).unwrap();
    let events = wait(&s, |e| matches!(e, Event::Exited { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Exited { code: 0, .. }))
    );
}
