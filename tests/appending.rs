use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tide_terminal::{
    append::{AppendMode, FollowUp, Prepared, append_file},
    capture::{CAPTURE_LIMIT, Recorder, Recording},
    session::{Event, Launch, Options, Session},
};
fn options(command: &str, cwd: &std::path::Path) -> Options {
    let mut options = Options::shell("/bin/bash", cwd.to_owned());
    options.private = true;
    options.launch = Launch::ShellCommand {
        shell: "/bin/bash".into(),
        command: command.into(),
    };
    options
}
fn finish(session: &Session) -> (String, u32) {
    let start = Instant::now();
    let mut text = String::new();
    while start.elapsed() < Duration::from_secs(8) {
        if let Ok(event) = session.events.recv_timeout(Duration::from_millis(25)) {
            match event {
                Event::Output { text: chunk } => text.push_str(&chunk),
                Event::Exited { code, .. } => return (text, code),
                _ => {}
            }
        }
    }
    panic!("Session did not finish: {text}")
}
fn recording(session: &Session) -> Recording {
    session
        .state
        .lock()
        .unwrap()
        .recording
        .clone()
        .expect("Missing command recording")
}
fn source(command: &str, dir: &std::path::Path) -> (Session, Recording) {
    let session = Session::spawn(options(command, dir)).unwrap();
    finish(&session);
    let record = recording(&session);
    (session, record)
}
fn prepare(record: Recording, mode: AppendMode, target: &str, dir: &std::path::Path) -> Prepared {
    FollowUp::new(record, mode, target.into(), options("unused", dir))
        .unwrap()
        .prepare()
        .unwrap()
        .unwrap()
}
#[test]
fn captured_pipe_is_finite_filtered_and_does_not_rerun_source() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source("printf x >> counter; printf 'hi\\nbye\\n'", dir.path());
    let Prepared::Command(options) = prepare(record, AppendMode::Pipe, "grep hi", dir.path())
    else {
        panic!()
    };
    let child = Session::spawn(*options).unwrap();
    let (text, code) = finish(&child);
    assert_eq!(code, 0);
    assert_eq!(text, "hi\r\n");
    assert_eq!(std::fs::read(dir.path().join("counter")).unwrap(), b"x");
}
#[test]
fn pending_action_waits_for_source_completion() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::spawn(options(
        "read answer; printf 'DONE:%s' \"$answer\"",
        dir.path(),
    ))
    .unwrap();
    let record = recording(&session);
    let request = FollowUp::new(
        record,
        AppendMode::Pipe,
        "cat".into(),
        options("unused", dir.path()),
    )
    .unwrap();
    assert!(request.prepare().unwrap().is_none());
    session.write(b"yes\n".to_vec()).unwrap();
    finish(&session);
    assert!(matches!(
        request.prepare().unwrap(),
        Some(Prepared::Command(_))
    ));
}
#[test]
fn after_success_skips_failed_or_interrupted_sources() {
    let dir = tempfile::tempdir().unwrap();
    for code in [0, 1, 42, 130] {
        let (_session, record) = source(&format!("exit {code}"), dir.path());
        let prepared = prepare(record, AppendMode::AfterSuccess, "printf NEXT", dir.path());
        match prepared {
            Prepared::Skipped(actual) => assert_eq!(actual, code),
            Prepared::Command(opts) => {
                assert_eq!(code, 0);
                assert_eq!(finish(&Session::spawn(*opts).unwrap()), ("NEXT".into(), 0));
            }
            _ => panic!(),
        }
    }
}
#[test]
fn failed_producer_can_still_be_filtered_or_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source("printf 'ERROR fixture\\n'; exit 7", dir.path());
    let Prepared::Command(opts) =
        prepare(record.clone(), AppendMode::Pipe, "grep ERROR", dir.path())
    else {
        panic!()
    };
    assert_eq!(
        finish(&Session::spawn(*opts).unwrap()),
        ("ERROR fixture\r\n".into(), 0)
    );
    let Prepared::File { path, output, .. } =
        prepare(record, AppendMode::File, "failed.txt", dir.path())
    else {
        panic!()
    };
    append_file(&path, &output).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "ERROR fixture\n");
}
#[test]
fn redirect_preserves_contents_and_treats_path_as_literal() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source("printf 'new\\n'", dir.path());
    let name = "saved output $(touch must-not-exist).txt";
    let path = dir.path().join(name);
    std::fs::write(&path, b"old\n").unwrap();
    let Prepared::File { path, output, .. } = prepare(record, AppendMode::File, name, dir.path())
    else {
        panic!()
    };
    append_file(&path, &output).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"old\nnew\n");
    assert!(!dir.path().join("must-not-exist").exists());
}
#[test]
fn redirect_creates_file_but_reports_invalid_parent() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(append_file(&dir.path().join("new"), b"text").unwrap(), 4);
    assert!(append_file(&dir.path().join("missing/file"), b"text").is_err());
    assert!(append_file(dir.path(), b"text").is_err());
}
#[test]
fn captured_input_reaches_entire_shell_pipeline_in_every_shell() {
    let dir = tempfile::tempdir().unwrap();
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let mut opts = options("grep foo | tr a-z A-Z && printf DONE", dir.path());
        opts.launch = Launch::ShellCommand {
            shell: shell.into(),
            command: "grep foo | tr a-z A-Z && printf DONE".into(),
        };
        opts.stdin = Some(b"foo\nbar\n".to_vec());
        assert_eq!(
            finish(&Session::spawn(opts).unwrap()),
            ("FOO\r\nDONE".into(), 0)
        );
    }
}
#[test]
fn capture_input_is_not_echoed_and_preserves_unicode_quotes_and_newlines() {
    let dir = tempfile::tempdir().unwrap();
    let input = "日本🙂\n'\"$()\\\n";
    let mut opts = options("cat", dir.path());
    opts.stdin = Some(input.as_bytes().to_vec());
    assert_eq!(
        finish(&Session::spawn(opts).unwrap()),
        (input.replace('\n', "\r\n"), 0)
    );
}
#[test]
fn capture_is_bounded_and_signals_truncation() {
    let mut recorder = Recorder::new(
        Some("fixture".into()),
        PathBuf::from("/tmp"),
        "/bin/bash".into(),
        true,
    );
    let mut parser = vte::Parser::new();
    parser.advance(&mut recorder, b"FIRST_LINE\n");
    let block = b"a repeated output line\n".repeat(100_000);
    parser.advance(&mut recorder, &block);
    parser.advance(&mut recorder, b"LAST_LINE\n");
    recorder.finish(0);
    let record = recorder.current.unwrap();
    let record = record.lock().unwrap();
    assert!(record.truncated);
    assert!(record.output.len() <= CAPTURE_LIMIT);
    assert!(!record.output.contains("FIRST_LINE"));
    assert!(record.output.ends_with("LAST_LINE\n"));
}
#[test]
fn command_boundaries_are_correct_even_with_everything_in_one_pty_read() {
    let mut recorder = Recorder::new(None, PathBuf::from("/tmp"), "/bin/bash".into(), true);
    let mut parser = vte::Parser::new();
    parser.advance(&mut recorder,b"prompt> typed text\x1b]777;tide;command;ZWNobyBmaXJzdA==\x07\x1b]133;C\x07\x1b[31mfirst\x1b[0m\r\n\x1b]133;D;0\x07prompt> ");
    let first = recorder.current.clone().unwrap();
    assert_eq!(first.lock().unwrap().output, "first\n");
    assert_eq!(first.lock().unwrap().command, "echo first");
    parser.advance(
        &mut recorder,
        b"\x1b]777;tide;command;ZWNobyBzZWNvbmQ=\x07\x1b]133;C\x07second\n\x1b]133;D;7\x07",
    );
    let second = recorder.current.clone().unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(first.lock().unwrap().output, "first\n");
    assert_eq!(first.lock().unwrap().exit_code, Some(0));
    assert_eq!(second.lock().unwrap().exit_code, Some(7));
}
#[test]
fn fish_start_before_command_marker_is_supported() {
    let mut recorder = Recorder::new(None, PathBuf::from("/tmp"), "/usr/bin/fish".into(), true);
    let mut parser = vte::Parser::new();
    for byte in b"\x1b]133;C\x07\x1b]777;tide;command;ZWNobyBoaQ==\x07hi\n\x1b]133;D;0\x07" {
        parser.advance(&mut recorder, &[*byte]);
    }
    let record = recorder.current.unwrap();
    let record = record.lock().unwrap();
    assert_eq!(record.command, "echo hi");
    assert_eq!(record.output, "hi\n");
}
#[test]
fn managed_capture_ignores_nested_shell_completion_markers() {
    let mut recorder = Recorder::new(
        Some("outer".into()),
        PathBuf::from("/tmp"),
        "/bin/bash".into(),
        false,
    );
    let mut parser = vte::Parser::new();
    parser.advance(&mut recorder, b"before\x1b]133;D;0\x07after");
    assert!(
        recorder
            .current
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .exit_code
            .is_none()
    );
    recorder.finish(42);
    assert_eq!(
        recorder.current.unwrap().lock().unwrap().output,
        "beforeafter"
    );
}
#[test]
fn fresh_followup_preserves_source_shell_directory_and_privacy() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source("printf hello", dir.path());
    let request = FollowUp::new(
        record,
        AppendMode::Pipe,
        "cat".into(),
        Options::shell("/bin/false", PathBuf::from("/")),
    )
    .unwrap();
    let Some(Prepared::Command(opts)) = request.prepare().unwrap() else {
        panic!()
    };
    assert_eq!(opts.cwd, dir.path());
    assert!(opts.private);
    assert_eq!(opts.shell_name(), "bash");
}
#[test]
fn empty_and_nul_append_targets_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source("true", dir.path());
    for mode in [AppendMode::AfterSuccess, AppendMode::Pipe, AppendMode::File] {
        for target in ["", "  ", "bad\0target"] {
            assert!(
                FollowUp::new(
                    record.clone(),
                    mode,
                    target.into(),
                    options("true", dir.path())
                )
                .is_err()
            );
        }
    }
}
#[test]
fn captured_stdin_is_rejected_for_an_interactive_shell() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::shell("/bin/bash", dir.path().to_owned());
    opts.stdin = Some(b"unsafe".to_vec());
    assert!(Session::spawn(opts).is_err());
}

#[test]
fn every_integrated_shell_captures_a_run_without_prompt_or_later_job_text() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = Options::shell(shell, dir.path().to_owned());
        opts.private = true;
        opts.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        let session = Session::spawn(opts).unwrap();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if matches!(
                session.events.recv_timeout(Duration::from_millis(50)),
                Ok(Event::Prompt)
            ) {
                break;
            }
        }
        session
            .write(b"printf 'CAPTURE_FIRST\\n'\n".to_vec())
            .unwrap();
        let wait_done = |session: &Session| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                if matches!(
                    session.events.recv_timeout(Duration::from_millis(50)),
                    Ok(Event::CommandEnded { .. })
                ) {
                    return;
                }
            }
            panic!("No shell completion")
        };
        wait_done(&session);
        let first = recording(&session);
        let text = first.lock().unwrap().output.clone();
        assert!(text.contains("CAPTURE_FIRST"), "{shell}: {text:?}");
        assert!(
            !text.contains("printf"),
            "{shell}: echoed command leaked into capture: {text:?}"
        );
        session
            .write(b"printf 'CAPTURE_SECOND\\n'\n".to_vec())
            .unwrap();
        wait_done(&session);
        let second = recording(&session);
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!first.lock().unwrap().output.contains("CAPTURE_SECOND"));
        session.write(b"exit\n".to_vec()).unwrap();
        finish(&session);
    }
}
#[test]
fn hidden_password_input_is_not_added_to_capture() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options("unused", dir.path());
    opts.launch = Launch::Command(vec![
        "python3".into(),
        "-u".into(),
        "-c".into(),
        "import getpass;getpass.getpass('Password: ');print('ACCEPTED')".into(),
    ]);
    let session = Session::spawn(opts).unwrap();
    let start = Instant::now();
    let mut prompted = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Event::Attention { .. }) = session.events.recv_timeout(Duration::from_millis(50))
        {
            prompted = true;
            break;
        }
    }
    assert!(prompted);
    session
        .write(b"capture-test-hidden-secret\n".to_vec())
        .unwrap();
    finish(&session);
    let record = recording(&session);
    let record = record.lock().unwrap();
    assert!(record.output.contains("ACCEPTED"));
    assert!(!record.output.contains("capture-test-hidden-secret"));
    assert!(record.private);
}
#[test]
fn temporary_captured_input_is_removed_when_session_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options("readlink /proc/self/fd/0", dir.path());
    opts.stdin = Some(b"captured".to_vec());
    let session = Session::spawn(opts).unwrap();
    let (text, code) = finish(&session);
    assert_eq!(code, 0);
    let path = PathBuf::from(text.trim());
    assert!(path.file_name().is_some_and(|name| name == "stdin"));
    drop(session);
    assert!(!path.exists());
}

#[test]
fn separate_stream_retention_limits_and_truncation_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, record) = source(
        "python3 -c 'import os;os.write(1,b\"KEEP\\n\");os.write(2,b\"ERROR_LINE\\n\"*120000+b\"ERR_TAIL\\n\")'",
        dir.path(),
    );
    let record = record.lock().unwrap();
    let streams = record.streams.as_ref().unwrap();
    assert_eq!(streams.stdout, "KEEP\n");
    assert!(!streams.stdout_truncated);
    assert!(streams.stderr.len() <= CAPTURE_LIMIT);
    assert!(streams.stderr.ends_with("ERR_TAIL\n"));
    assert!(streams.stderr_truncated);
}
#[test]
fn hidden_password_keystrokes_are_absent_from_each_separate_stream() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::spawn(options(
        "python3 -c 'import getpass;getpass.getpass(\"Password: \");print(\"ACCEPTED\")'",
        dir.path(),
    ))
    .unwrap();
    let start = Instant::now();
    let mut prompted = false;
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(
            session.events.recv_timeout(Duration::from_millis(50)),
            Ok(Event::Attention { .. })
        ) {
            prompted = true;
            break;
        }
    }
    assert!(prompted);
    session.write(b"hidden-split-secret\n".to_vec()).unwrap();
    finish(&session);
    let record = recording(&session);
    let record = record.lock().unwrap();
    let streams = record.streams.as_ref().unwrap();
    assert!(streams.stdout.contains("ACCEPTED"));
    assert!(!streams.stdout.contains("hidden-split-secret"));
    assert!(!streams.stderr.contains("hidden-split-secret"));
    assert!(record.private);
}
