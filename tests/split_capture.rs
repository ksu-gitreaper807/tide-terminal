use std::path::PathBuf;
use tide_terminal::split_capture::{Channel, SplitCapture};
fn capture() -> SplitCapture {
    SplitCapture::new(None, PathBuf::from("/tmp"), "/bin/bash".into(), true)
}
fn feed(c: &mut SplitCapture, p: &mut [vte::Parser; 2], channel: Channel, bytes: &[u8]) {
    c.select(channel);
    p[channel as usize].advance(c, bytes);
}
#[test]
fn completion_waits_for_both_streams_and_excludes_their_prompts() {
    let mut c = capture();
    let mut p = std::array::from_fn(|_| vte::Parser::new());
    feed(
        &mut c,
        &mut p,
        Channel::Stdout,
        b"\x1b]133;C\x07OUT\n\x1b]133;D;7\x07OUT_PROMPT",
    );
    assert!(c.completed.lock().unwrap().is_none());
    feed(
        &mut c,
        &mut p,
        Channel::Stderr,
        b"\x1b]133;C\x07ERR\n\x1b]133;D;7\x07ERR_PROMPT",
    );
    let record = c.completed.lock().unwrap();
    let record = record.as_ref().unwrap();
    let streams = record.streams.as_ref().unwrap();
    assert_eq!(record.exit_code, Some(7));
    assert_eq!(streams.stdout, "OUT\n");
    assert_eq!(streams.stderr, "ERR\n");
    assert!(!record.output.contains("PROMPT"));
}
#[test]
fn fast_next_command_does_not_reassign_delayed_stderr_to_the_new_command() {
    let mut c = capture();
    let mut p = std::array::from_fn(|_| vte::Parser::new());
    feed(
        &mut c,
        &mut p,
        Channel::Stdout,
        b"\x1b]133;C\x07ONE\x1b]133;D;0\x07\x1b]133;C\x07TWO\x1b]133;D;0\x07",
    );
    feed(
        &mut c,
        &mut p,
        Channel::Stderr,
        b"\x1b]133;C\x07ERR_ONE\x1b]133;D;0\x07\x1b]133;C\x07ERR_TWO\x1b]133;D;0\x07",
    );
    let record = c.completed.lock().unwrap();
    let streams = record.as_ref().unwrap().streams.as_ref().unwrap();
    assert_eq!(streams.stdout, "TWO");
    assert_eq!(streams.stderr, "ERR_TWO");
}
#[test]
fn independent_chunk_boundaries_preserve_valid_utf8_and_control_sequences() {
    for stride in [1, 3, 11, 8192] {
        let mut c = capture();
        let mut p = std::array::from_fn(|_| vte::Parser::new());
        let a = "\x1b]133;C\x07\x1b[31m日本🙂\x1b[0m\n\x1b]133;D;0\x07";
        let b = "\x1b]133;C\x07\x1b[32mcafé\x1b[0m\n\x1b]133;D;0\x07";
        let mut a = a.as_bytes().chunks(stride);
        let mut b = b.as_bytes().chunks(stride);
        loop {
            let x = a.next();
            let y = b.next();
            if x.is_none() && y.is_none() {
                break;
            }
            if let Some(x) = x {
                feed(&mut c, &mut p, Channel::Stdout, x);
            }
            if let Some(y) = y {
                feed(&mut c, &mut p, Channel::Stderr, y);
            }
        }
        let record = c.completed.lock().unwrap();
        let streams = record.as_ref().unwrap().streams.as_ref().unwrap();
        assert_eq!(streams.stdout, "日本🙂\n");
        assert_eq!(streams.stderr, "café\n");
    }
}
#[test]
fn wrapper_actions_leave_the_completed_source_available() {
    let mut c = capture();
    let mut p = std::array::from_fn(|_| vte::Parser::new());
    for channel in [Channel::Stdout, Channel::Stderr] {
        feed(
            &mut c,
            &mut p,
            channel,
            b"\x1b]133;C\x07SOURCE\x1b]133;D;0\x07",
        );
    }
    for channel in [Channel::Stdout, Channel::Stderr] {
        feed(
            &mut c,
            &mut p,
            channel,
            b"\x1b]133;C\x07\x1b]777;tide;append\x07WRAPPER\x1b]133;D;0\x07",
        );
    }
    let record = c.completed.lock().unwrap();
    let record = record.as_ref().unwrap();
    assert!(!record.output.contains("WRAPPER"));
    assert_eq!(record.streams.as_ref().unwrap().stderr, "SOURCE");
    assert!(c.ready.load(std::sync::atomic::Ordering::Acquire));
}
#[test]
fn unmatched_command_boundaries_are_bounded_and_old_recordings_fail_explicitly() {
    let mut c = capture();
    let mut p = std::array::from_fn(|_| vte::Parser::new());
    feed(
        &mut c,
        &mut p,
        Channel::Stdout,
        b"\x1b]133;C\x07old\x1b]133;D;0\x07",
    );
    let old = c.current.clone().unwrap();
    for _ in 1..100 {
        feed(
            &mut c,
            &mut p,
            Channel::Stdout,
            b"\x1b]133;C\x07new\x1b]133;D;0\x07",
        );
    }
    assert_eq!(old.lock().unwrap().exit_code, Some(-1));
    let follow = tide_terminal::append::FollowUp::new(
        old,
        tide_terminal::append::AppendMode::Pipe,
        "cat".into(),
        tide_terminal::session::Options::shell("/bin/bash", PathBuf::from("/tmp")),
    )
    .unwrap();
    assert!(follow.prepare().is_err());
    for _ in 0..100 {
        feed(
            &mut c,
            &mut p,
            Channel::Stderr,
            b"\x1b]133;C\x07err\x1b]133;D;0\x07",
        );
    }
    let record = c.completed.lock().unwrap();
    let streams = record.as_ref().unwrap().streams.as_ref().unwrap();
    assert_eq!(streams.stdout, "new");
    assert_eq!(streams.stderr, "err");
}
