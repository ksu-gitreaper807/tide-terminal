//! A real child asks terminal questions and checks the bytes returned on its own PTY.
use std::time::{Duration, Instant};
use tide_terminal::{
    rules::RuleSpec,
    session::{Event, Launch, Options, Session},
};

fn query(stream: &[u8], expected: &[u8]) {
    let stream = serde_json::to_string(stream).unwrap();
    let expected = serde_json::to_string(expected).unwrap();
    let script = format!(
        r#"
import os,select,time,tty
tty.setraw(0)
os.write(1,bytes({stream}))
expected=bytes({expected})
reply=b''
deadline=time.monotonic()+2
while len(reply)<len(expected) and time.monotonic()<deadline:
    if select.select([0],[],[],.05)[0]: reply+=os.read(0,4096)
assert reply==expected,(reply,expected)
os.write(1,b'QUERY_OK')
"#
    );
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::shell("/bin/bash", dir.path().to_path_buf());
    opts.launch = Launch::Command(vec![
        "/usr/bin/python3".into(),
        "-u".into(),
        "-c".into(),
        script,
    ]);
    opts.rows = 10;
    opts.cols = 20;
    opts.private = true;
    opts.rules = RuleSpec {
        timeout: Some(Duration::from_secs(4)),
        grace: Duration::from_millis(100),
        force: true,
        ..Default::default()
    };
    let session = Session::spawn(opts).unwrap();
    let start = Instant::now();
    let mut output = String::new();
    let mut code = None;
    while start.elapsed() < Duration::from_secs(6) {
        if let Ok(event) = session.events.recv_timeout(Duration::from_millis(50)) {
            match event {
                Event::Output { text } => output.push_str(&text),
                Event::Exited { code: exit, .. } => {
                    code = Some(exit);
                    break;
                }
                _ => {}
            }
        }
    }
    assert_eq!(code, Some(0), "Query dialogue failed: {output}");
    assert!(output.contains("QUERY_OK"), "{output}");
}
#[test]
fn dsr_roundtrip() {
    query(b"\x1b[3;4H\x1b[6n", b"\x1b[3;4R");
}
#[test]
fn device_status_roundtrip() {
    query(b"\x1b[5n", b"\x1b[0n");
}
#[test]
fn primary_attributes_roundtrip() {
    query(b"\x1b[c", b"\x1b[?6c");
}
#[test]
fn text_area_size_roundtrip() {
    query(b"\x1b[18t", b"\x1b[8;10;20t");
}
#[test]
fn foreground_color_roundtrip() {
    query(b"\x1b]10;?\x07", b"\x1b]10;rgb:d5d5/dcdc/e5e5\x07");
}
#[test]
fn background_color_roundtrip() {
    query(b"\x1b]11;?\x07", b"\x1b]11;rgb:0f0f/1414/1c1c\x07");
}
#[test]
fn indexed_256_color_roundtrip() {
    query(b"\x1b]4;196;?\x07", b"\x1b]4;196;rgb:ffff/0000/0000\x07");
}
#[test]
fn cursor_color_roundtrip() {
    query(b"\x1b]12;?\x07", b"\x1b]12;rgb:7272/cbcb/c6c6\x07");
}
#[test]
fn explicitly_set_color_roundtrip() {
    query(
        b"\x1b]4;196;rgb:12/34/56\x07\x1b]4;196;?\x07",
        b"\x1b]4;196;rgb:1212/3434/5656\x07",
    );
}
