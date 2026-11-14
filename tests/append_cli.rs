use std::{
    io::Read,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tide_terminal::{
    append_cli::{Action, SnapshotServer, Stream},
    capture::{CAPTURE_LIMIT, CapturedRun},
    session::{Event, Launch, Options, Session},
};

const SHELLS: [&str; 3] = ["/bin/bash", "/bin/zsh", "/usr/bin/fish"];

#[test]
fn printed_fake_metadata_cannot_turn_failure_into_success_or_change_directory() {
    for shell_path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(shell_path, dir.path());
        let script = "import os; payload=b'\\x1b]133;D;0\\x07\\x1b]133;A\\x07\\x1b]7;file://host/forged\\x07\\x1b]777;tide;cwd;L2ZvcmdlZA==\\x07\\x1b]777;tide;command;Zm9yZ2Vk\\x07\\x1b]777;tide;append\\x07\\x1b]777;tide;v1;wrong;end;0\\x07'; os.write(1,payload+b'ACTUAL_OUT\\n'); os.write(2,payload+b'ACTUAL_ERR\\n'); raise SystemExit(7)";
        let command = format!("python3 -c {}", quote(script));
        assert_eq!(run(&s, &command).0, 7, "{shell_path}");
        let record = s.state.lock().unwrap().recording.clone().unwrap();
        {
            let record = record.lock().unwrap();
            assert_eq!(record.exit_code, Some(7));
            assert_eq!(record.command, command);
            assert_eq!(record.cwd, dir.path());
            let streams = record.streams.as_ref().unwrap();
            assert_eq!(streams.stdout, "ACTUAL_OUT\n");
            assert_eq!(streams.stderr, "ACTUAL_ERR\n");
        }
        assert_eq!(run(&s, "A_: '&& touch SHOULD_NOT_EXIST'").0, 7);
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
        assert_eq!(run(&s, "A_: '2>> errors.txt'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("errors.txt")).unwrap(),
            "ACTUAL_ERR\n"
        );
    }
}
fn shell(path: &str, cwd: &Path) -> Session {
    let mut o = Options::shell(path, cwd.to_owned());
    o.launch = Launch::Shell {
        path: path.into(),
        clean: true,
    };
    o.private = true;
    o.env.push((
        "TIDE_EXE".into(),
        std::env::var("TIDE_APPEND_TEST_BINARY")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_tide-terminal").into()),
    ));
    let s = Session::spawn(o).unwrap();
    until(&s, |e| matches!(e, Event::Prompt));
    s
}
fn until(s: &Session, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let started = Instant::now();
    let mut events = vec![];
    while started.elapsed() < Duration::from_secs(15) {
        if let Ok(event) = s.events.recv_timeout(Duration::from_millis(25)) {
            let finished = done(&event);
            events.push(event);
            if finished {
                return events;
            }
        }
    }
    panic!("No expected shell event: {events:?}")
}
fn run(s: &Session, command: &str) -> (i32, String) {
    // A previous completion may have emitted Prompt in the same PTY read.
    while s.events.try_recv().is_ok() {}
    s.write(format!("{command}\n").into_bytes()).unwrap();
    let events = until(s, |e| matches!(e, Event::CommandEnded { .. }));
    let code = events
        .iter()
        .find_map(|e| {
            if let Event::CommandEnded { code, .. } = e {
                Some(*code)
            } else {
                None
            }
        })
        .unwrap();
    let text = events
        .iter()
        .filter_map(|e| {
            if let Event::Output { text } = e {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    (code, text)
}
fn captured(s: &Session) -> String {
    s.state
        .lock()
        .unwrap()
        .recording
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .output
        .clone()
}
fn quote(s: &str) -> String {
    let mut result = String::from("\"");
    for c in s.chars() {
        if "\\\"$`".contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result.push('"');
    result
}

#[test]
fn nested_append_uses_pipeline_input_and_keeps_the_original_recording() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(run(&s, "printf ORIGINAL; printf x >> source-runs").0, 0);
        let action = "&& printf 'kushal\\nother\\n' | A_: '| grep k > filtered'";
        assert_eq!(run(&s, &format!("A_: {}", quote(action))).0, 0, "{path}");
        assert_eq!(
            std::fs::read(dir.path().join("filtered")).unwrap(),
            b"kushal\n",
            "{path}"
        );
        assert_eq!(run(&s, "A_: '>> original'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("original")).unwrap(),
            b"ORIGINAL",
            "{path}"
        );
        assert_eq!(std::fs::read(dir.path().join("source-runs")).unwrap(), b"x");
    }
}

#[test]
fn nested_timer_and_append_helpers_use_the_running_binary_in_every_shell() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(run(&s, "true").0, 0);
        let body = "printf 'kushal\\nother\\n' | A_: '| grep k > timed'";
        // Each shell receives exactly one fragment and the timer receives exactly
        // one command, with quotes retained for its next shell evaluation.
        let action = format!("&& T_: {} : -s 2", quote(body));
        assert_eq!(run(&s, &format!("A_: {}", quote(&action))).0, 0, "{path}");
        assert_eq!(
            std::fs::read(dir.path().join("timed")).unwrap(),
            b"kushal\n",
            "{path}"
        );
    }
}

#[test]
fn nested_append_streams_files_failures_and_multiple_stages() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(
            run(
                &s,
                "python3 -c 'import os;os.write(1,b\"OUT\\n\");os.write(2,b\"ERR\\n\")'"
            )
            .0,
            0
        );
        for (action, status) in [
            ("1| A_: '>> selected'", 0),
            ("2| A_: '| cat > errors'", 0),
            ("&& printf '日本🙂' | A_: '>> \"space file\"'", 0),
            ("&& printf 'café' | A_: '>> \"space file\"'", 0),
            (
                "&& printf 'kushal\\nother\\n' | A_: '| A_: \"| grep k > deep\"'",
                0,
            ),
            ("&& printf value | A_: '| grep absent'", 1),
            ("&& printf value | A_: '| cat > /dev/null; exit 42'", 42),
            ("&& printf value | A_: '>> missing/child'", 1),
            ("&& printf value | A_: '2>> ambiguous'", 1),
        ] {
            assert_eq!(
                run(&s, &format!("A_: {}", quote(action))).0,
                status,
                "{path}: {action}"
            );
        }
        for (file, expected) in [
            ("selected", "OUT\n"),
            ("errors", "ERR\n"),
            ("space file", "日本🙂café"),
            ("deep", "kushal\n"),
        ] {
            assert_eq!(
                std::fs::read_to_string(dir.path().join(file)).unwrap(),
                expected,
                "{path}: {file}"
            );
        }
        assert!(!dir.path().join("ambiguous").exists());
        assert_eq!(run(&s, "A_: '>> unchanged'").0, 0);
        let original = std::fs::read_to_string(dir.path().join("unchanged")).unwrap();
        assert!(
            original.contains("OUT\n") && original.contains("ERR\n"),
            "{path}: {original:?}"
        );
    }
}

#[test]
fn nested_timer_enforces_its_deadline_and_preserves_the_source() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(run(&s, "printf SOURCE").0, 0);
        let action = "&& T_: 'sleep 20' : -s 0.15 -grace 100ms";
        let started = Instant::now();
        assert_ne!(run(&s, &format!("A_: {}", quote(action))).0, 0, "{path}");
        assert!(started.elapsed() < Duration::from_secs(4), "{path}");
        assert_eq!(run(&s, "A_: '>> original'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("original")).unwrap(),
            b"SOURCE"
        );
    }
}

#[test]
fn nested_pipeline_streams_past_the_capture_limit_and_accepts_an_early_consumer() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "true");
        let producer = "import os;os.write(1,b'x'*(2*1024*1024))";
        let action = format!("&& python3 -c {} | A_: '>> large'", quote(producer));
        assert_eq!(run(&s, &format!("A_: {}", quote(&action))).0, 0, "{path}");
        assert_eq!(
            std::fs::read(dir.path().join("large")).unwrap(),
            vec![b'x'; 2 * 1024 * 1024]
        );
        // An unbounded producer must stop on the consumer's closed pipe. Buffering
        // stdin to EOF before launching head would hang this regression.
        let producer =
            "import os\ntry:\n while True: os.write(1,b'x'*32768)\nexcept BrokenPipeError: pass";
        std::fs::write(dir.path().join("producer.py"), producer).unwrap();
        let action = "&& python3 producer.py | A_: '| head -c 1 > first'";
        assert_eq!(run(&s, &format!("A_: {}", quote(action))).0, 0, "{path}");
        assert_eq!(std::fs::read(dir.path().join("first")).unwrap(), b"x");
    }
}

#[test]
fn syntax_accepts_pipe_success_and_single_literal_append_paths() {
    for (text, expected) in [
        (
            " | grep -i \"hello\" ",
            Action::Pipe("grep -i \"hello\"".into(), Stream::Combined),
        ),
        ("&& echo ok", Action::AfterSuccess("echo ok".into())),
        (
            ">>test.txt",
            Action::File("test.txt".into(), Stream::Combined),
        ),
        (
            ">> \"space file.txt\"",
            Action::File("space file.txt".into(), Stream::Combined),
        ),
        (
            ">> '$(touch injected); log'",
            Action::File("$(touch injected); log".into(), Stream::Combined),
        ),
        (
            ">> escaped\\ name",
            Action::File("escaped name".into(), Stream::Combined),
        ),
        (
            ">> '日本🙂.log'",
            Action::File("日本🙂.log".into(), Stream::Combined),
        ),
    ] {
        assert_eq!(Action::parse(text).unwrap(), expected, "{text}");
    }
}
#[test]
fn syntax_rejects_ambiguous_redirections_and_unavailable_streams() {
    for text in [
        "",
        "|",
        "&&",
        ">>",
        ">> ''",
        ">> a b",
        ">> a;touch bad",
        ">> a 2>&1",
        ">> 'oops",
        ">> a\\",
        "|| true",
        "|& cat",
        "2>&1 test.txt",
        "2>>",
        "1|",
        "> out",
        "| cat\0",
    ] {
        assert!(Action::parse(text).is_err(), "{text:?}");
    }
}
#[test]
fn socket_is_private_bounded_to_session_and_removed_on_drop() {
    let completed = Arc::new(Mutex::new(None));
    let a = SnapshotServer::new(completed.clone()).unwrap();
    let b = SnapshotServer::new(Arc::new(Mutex::new(None))).unwrap();
    assert_eq!(
        std::fs::metadata(a.path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    *completed.lock().unwrap() = Some(CapturedRun {
        command: "source".into(),
        cwd: "/tmp".into(),
        shell: "/bin/bash".into(),
        private: true,
        output: "private-result".into(),
        streams: None,
        truncated: false,
        exit_code: Some(0),
    });
    for (server, expected) in [(&a, Some("private-result")), (&b, None)] {
        let mut bytes = vec![];
        UnixStream::connect(&server.path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let record: Option<CapturedRun> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(record.as_ref().map(|r| r.output.as_str()), expected);
    }
    let path = a.path.clone();
    drop(a);
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}
#[test]
fn every_shell_pipes_filters_chains_and_never_reruns_the_source() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(
            run(
                &s,
                "printf x >> counter; printf 'Hello\\nbye\\nHELLO again\\n'"
            )
            .0,
            0
        );
        let original = captured(&s);
        assert_eq!(
            run(
                &s,
                "A_: '| grep -i hello | sort > filtered && printf PIPE_DONE'"
            )
            .0,
            0,
            "{path}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("filtered")).unwrap(),
            "HELLO again\nHello\n"
        );
        assert_eq!(captured(&s), original);
        assert_eq!(run(&s, "A_: '>> saved'").0, 0);
        assert_eq!(run(&s, "A_: '>> saved'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("saved")).unwrap(),
            original.repeat(2)
        );
        assert_eq!(std::fs::read(dir.path().join("counter")).unwrap(), b"x");
    }
}
#[test]
fn every_shell_reports_empty_source_then_uses_next_completed_command() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        let (code, text) = run(&s, "A_: '>> absent'");
        assert_ne!(code, 0);
        assert!(text.contains("No completed command"), "{path}: {text}");
        assert!(!dir.path().join("absent").exists());
        run(&s, "printf FIRST");
        run(&s, "A_: '>> first'");
        run(&s, "printf SECOND");
        run(&s, "A_: '>> second'");
        assert_eq!(std::fs::read(dir.path().join("first")).unwrap(), b"FIRST");
        assert_eq!(std::fs::read(dir.path().join("second")).unwrap(), b"SECOND");
    }
}
#[test]
fn every_shell_combines_real_stdout_stderr_and_keeps_failed_output() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(
            &s,
            "python3 -c 'import os;os.write(1,b\"OUT\\n\");os.write(2,b\"ERR\\n\");raise SystemExit(7)'",
        );
        let original = captured(&s);
        assert!(original.contains("OUT\n"));
        assert!(original.contains("ERR\n"));
        assert_eq!(run(&s, "A_: '&& touch should-not-exist'").0, 7);
        assert!(!dir.path().join("should-not-exist").exists());
        assert_eq!(run(&s, "A_: '>> both'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("both")).unwrap(),
            original
        );
        assert_eq!(run(&s, "A_: '| grep ERR > errors'").0, 0);
        assert_eq!(std::fs::read(dir.path().join("errors")).unwrap(), b"ERR\n");
        assert_eq!(run(&s, "A_: '2>> stderr-only'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("stderr-only")).unwrap(),
            b"ERR\n"
        );
    }
}
#[test]
fn every_shell_preserves_pipe_exit_status_and_success_followups() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "printf source");
        assert_eq!(run(&s, "A_: '| grep absent'").0, 1);
        assert_eq!(run(&s, "A_: '| cat > /dev/null; exit 42'").0, 42);
        assert_eq!(run(&s, "A_: '&& printf ok > success'").0, 0);
        assert_eq!(std::fs::read(dir.path().join("success")).unwrap(), b"ok");
    }
}
#[test]
fn every_shell_append_handles_normalized_output_type_matrix() {
    let cases: [(&str, &[u8], &str); 15] = [
        ("empty", b"", ""),
        ("no-newline", b"last", "last"),
        ("blank-lines", b"\n\n", "\n\n"),
        ("multiline", b"one\ntwo\n", "one\ntwo\n"),
        ("unicode", "日本 café 🙂\n".as_bytes(), "日本 café 🙂\n"),
        ("json", b"{\"a\":[1,true]}\n", "{\"a\":[1,true]}\n"),
        ("csv", b"a,b\n1,2\n", "a,b\n1,2\n"),
        ("trailing-space", b"x  \n", "x  \n"),
        ("ansi", b"\x1b[31mred\x1b[0m\n", "red\n"),
        ("progress", b"old\rnew\n", "new\n"),
        ("nul", b"a\0b\n", "ab\n"),
        ("backspace", b"ab\x08c\n", "ac\n"),
        ("tabs", b"a\tb\n", "a       b\n"),
        ("crlf", b"a\r\nb\r\n", "a\nb\n"),
        ("invalid-utf8", b"a\xffb\n", "a�b\n"),
    ];
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        for (name, bytes, expected) in cases {
            std::fs::write(dir.path().join("input"), bytes).unwrap();
            assert_eq!(run(&s, "cat input").0, 0);
            assert_eq!(captured(&s), expected, "{path}: {name}");
            let target = format!("result-{name}");
            std::fs::write(dir.path().join(&target), b"PREFIX\0").unwrap();
            for _ in 0..2 {
                assert_eq!(
                    run(&s, &format!("A_: '>> {target}'")).0,
                    0,
                    "{path}: {name}"
                );
            }
            let mut expected_bytes = b"PREFIX\0".to_vec();
            expected_bytes.extend(expected.repeat(2).as_bytes());
            assert_eq!(
                std::fs::read(dir.path().join(&target)).unwrap(),
                expected_bytes,
                "{path}: {name}"
            );
            assert_eq!(run(&s, "A_: '| cat > piped'").0, 0);
            assert_eq!(
                std::fs::read(dir.path().join("piped")).unwrap(),
                expected.as_bytes(),
                "{path}: {name}"
            );
        }
    }
}
#[test]
fn every_shell_large_capture_reports_truncation_and_small_consumer_reaches_eof() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(
            &s,
            "python3 -c 'import sys;sys.stdout.write(\"line\\n\"*250000+\"TAIL\\n\")'",
        );
        let source = captured(&s);
        assert!(source.len() <= CAPTURE_LIMIT);
        assert!(source.ends_with("TAIL\n"));
        assert!(
            s.state
                .lock()
                .unwrap()
                .recording
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .truncated
        );
        let (code, text) = run(&s, "A_: '>> large'");
        assert_eq!(code, 0);
        assert!(text.contains("retained tail"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("large")).unwrap(),
            source
        );
        assert_eq!(run(&s, "A_: '| head -c 1 > first-byte'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("first-byte")).unwrap(),
            source.as_bytes()[..1]
        );
        assert_eq!(run(&s, "A_: '| cat | wc -c > count'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("count"))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap(),
            source.len()
        );
    }
}
#[test]
fn every_shell_file_paths_are_literal_and_failures_preserve_source_and_files() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "printf payload");
        let name = "$(touch injected); 日本 log";
        let action = format!(">> {}", quote(name));
        assert_eq!(run(&s, &format!("A_: {}", quote(&action))).0, 0);
        assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"payload");
        assert!(!dir.path().join("injected").exists());
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        assert_eq!(run(&s, "A_: '>> link'").0, 0);
        assert_eq!(std::fs::read(dir.path().join("real")).unwrap(), b"payload");
        for target in [
            ">> missing/child",
            ">> .",
            ">> /dev/full",
            ">> a;touch injected",
            "2>&1 test.txt",
        ] {
            assert_ne!(
                run(&s, &format!("A_: {}", quote(target))).0,
                0,
                "{path}: {target}"
            );
            assert_eq!(captured(&s), "payload");
        }
        assert!(!dir.path().join("injected").exists());
    }
}
#[test]
fn terminals_cannot_consume_each_others_latest_output() {
    let dir = tempfile::tempdir().unwrap();
    let a = shell("/bin/bash", dir.path());
    let b = shell("/bin/bash", dir.path());
    run(&a, "printf A_SECRET");
    run(&b, "printf B_SECRET");
    run(&a, "A_: '>> a'");
    run(&b, "A_: '>> b'");
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"A_SECRET");
    assert_eq!(std::fs::read(dir.path().join("b")).unwrap(), b"B_SECRET");
}

#[test]
fn every_shell_supports_stderr_only_output_and_empty_pipeline_eof() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(
            &s,
            "python3 -c 'import os;os.write(2,\"ERR_日本\\n\".encode())'",
        );
        assert_eq!(run(&s, "A_: '>> stderr-text'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stderr-text")).unwrap(),
            "ERR_日本\n"
        );
        run(&s, "true");
        assert_eq!(run(&s, "A_: '| wc -c > empty-count'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("empty-count"))
                .unwrap()
                .trim(),
            "0"
        );
    }
}
#[test]
fn every_shell_uses_recorded_directory_and_rejects_read_only_target() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let s = shell(path, dir.path());
        run(&s, "cd sub; printf payload");
        assert_eq!(run(&s, "A_: '>> relative'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("sub/relative")).unwrap(),
            b"payload"
        );
        assert_eq!(run(&s, "A_: '| pwd > directory'").0, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub/directory"))
                .unwrap()
                .trim(),
            dir.path().join("sub").to_str().unwrap()
        );
        let target = dir.path().join("sub/readonly");
        std::fs::write(&target, b"PRESERVE").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert_ne!(run(&s, "A_: '>> readonly'").0, 0);
        assert_eq!(std::fs::read(&target).unwrap(), b"PRESERVE");
    }
}
#[test]
fn every_shell_interrupts_appended_pipeline_and_preserves_original_for_retry() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "printf original");
        s.write(b"A_: '| cat > drained; touch ready; sleep 30'\n".to_vec())
            .unwrap();
        let start = Instant::now();
        while !dir.path().join("ready").exists() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        s.write(vec![3]).unwrap();
        let events = until(&s, |e| matches!(e, Event::CommandEnded { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::CommandEnded{code,..} if *code!=0)),
            "{path}: {events:?}"
        );
        assert_eq!(run(&s, "A_: '>> retry'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("retry")).unwrap(),
            b"original"
        );
    }
}
#[test]
fn standalone_launcher_requires_session_and_works_inside_integrated_shell() {
    use std::process::Command;
    let launcher = Path::new(env!("CARGO_MANIFEST_DIR")).join("A_:");
    let result = Command::new(&launcher)
        .arg(">> unused")
        .env(
            "TIDE_EXE",
            std::env::var("TIDE_APPEND_TEST_BINARY")
                .unwrap_or_else(|_| env!("CARGO_BIN_EXE_tide-terminal").into()),
        )
        .env_remove("TIDE_APPEND_SOCKET")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("integrated Tide shell"));
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "printf standalone");
        assert_eq!(
            run(
                &s,
                &format!("{} '>> standalone'", quote(launcher.to_str().unwrap()))
            )
            .0,
            0
        );
        assert_eq!(
            std::fs::read(dir.path().join("standalone")).unwrap(),
            b"standalone"
        );
        assert_eq!(run(&s, "A_: '>> again'").0, 0);
        assert_eq!(
            std::fs::read(dir.path().join("again")).unwrap(),
            b"standalone"
        );
    }
}

#[test]
fn every_shell_can_select_each_stream_for_files_and_pipes_after_completion() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        assert_eq!(run(&s,"python3 -c 'import os;os.write(1,\"OUT_日本\\n\".encode());os.write(2,\"ERR_café🙂\\n\".encode());raise SystemExit(9)'").0,9);
        for (action, file, expected) in [
            ("1>> out", "out", "OUT_日本\n"),
            ("2>> err", "err", "ERR_café🙂\n"),
            ("1| cat > piped-out", "piped-out", "OUT_日本\n"),
            ("2| cat > piped-err", "piped-err", "ERR_café🙂\n"),
        ] {
            assert_eq!(
                run(&s, &format!("A_: {}", quote(action))).0,
                0,
                "{path}: {action}"
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join(file)).unwrap(),
                expected,
                "{path}: {action}"
            );
        }
    }
}
#[test]
fn every_shell_keeps_empty_streams_and_missing_newlines_separate() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        for (source, out, err) in [
            ("printf out", "out", ""),
            ("printf err >&2", "", "err"),
            ("true", "", ""),
            (
                "printf 'a\\nb\\n'; printf 'x\\ny\\n' >&2",
                "a\nb\n",
                "x\ny\n",
            ),
        ] {
            run(&s, source);
            for (action, file, expected) in
                [("1| cat > out", "out", out), ("2| cat > err", "err", err)]
            {
                assert_eq!(
                    run(&s, &format!("A_: {}", quote(action))).0,
                    0,
                    "{path}: {source}"
                );
                assert_eq!(
                    std::fs::read_to_string(dir.path().join(file)).unwrap(),
                    expected,
                    "{path}: {source}: {file}"
                );
            }
        }
    }
}
#[test]
fn explicit_original_redirections_determine_the_captured_channel() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(&s, "sh -c 'printf OUT; printf ERR >&2' 2>&1");
        run(&s, "A_: '1| cat > merged'");
        run(&s, "A_: '2| cat > empty'");
        assert_eq!(std::fs::read(dir.path().join("merged")).unwrap(), b"OUTERR");
        assert_eq!(std::fs::read(dir.path().join("empty")).unwrap(), b"");
        run(&s, "sh -c 'printf OUT; printf ERR >&2' 2> original-errors");
        run(&s, "A_: '1| cat > captured-out'");
        run(&s, "A_: '2| cat > captured-err'");
        assert_eq!(
            std::fs::read(dir.path().join("captured-out")).unwrap(),
            b"OUT"
        );
        assert_eq!(std::fs::read(dir.path().join("captured-err")).unwrap(), b"");
        assert_eq!(
            std::fs::read(dir.path().join("original-errors")).unwrap(),
            b"ERR"
        );
    }
}
#[test]
fn concurrent_writers_are_isolated_and_both_channels_are_drained() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        let program = "import os,threading; a=threading.Thread(target=lambda:[os.write(1,b'OUT\\n') for _ in range(20000)]); b=threading.Thread(target=lambda:[os.write(2,b'ERR\\n') for _ in range(20000)]); a.start(); b.start(); a.join(); b.join()";
        run(&s, &format!("python3 -c {}", quote(program)));
        for (action, file, expected) in [
            ("1| cat > out", "out", "OUT\n".repeat(20000)),
            ("2| cat > err", "err", "ERR\n".repeat(20000)),
        ] {
            assert_eq!(run(&s, &format!("A_: {}", quote(action))).0, 0, "{path}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join(file)).unwrap(),
                expected,
                "{path}: {file}"
            );
        }
    }
}
#[test]
fn both_streams_are_terminals_resized_with_the_controlling_terminal() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        s.resize(37, 123);
        run(
            &s,
            "python3 -c 'import os,json;print(json.dumps({\"tty\":[os.isatty(i) for i in range(3)],\"size\":[list(os.get_terminal_size(i)) for i in range(3)],\"foreground\":os.tcgetpgrp(0)==os.getpgrp()}))'",
        );
        run(&s, "A_: '1| cat > tty.json'");
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("tty.json")).unwrap()).unwrap();
        assert_eq!(
            result["tty"],
            serde_json::json!([true, true, true]),
            "{path}"
        );
        assert_eq!(
            result["size"],
            serde_json::json!([[123, 37], [123, 37], [123, 37]]),
            "{path}"
        );
        assert_eq!(result["foreground"], true, "{path}");
    }
}
#[test]
fn each_stream_uses_its_own_normalization_state() {
    for path in SHELLS {
        let dir = tempfile::tempdir().unwrap();
        let s = shell(path, dir.path());
        run(
            &s,
            "python3 -c 'import os;os.write(1,b\"old\\rnew\\n\\x1b[31mOUT\\x1b[0m\\n\");os.write(2,b\"bad\\rERR\\n\\x1b[32merror\\x1b[0m\\n\")'",
        );
        run(&s, "A_: '1| cat > out'");
        run(&s, "A_: '2| cat > err'");
        assert_eq!(
            std::fs::read(dir.path().join("out")).unwrap(),
            b"new\nOUT\n",
            "{path}"
        );
        assert_eq!(
            std::fs::read(dir.path().join("err")).unwrap(),
            b"ERR\nerror\n",
            "{path}"
        );
    }
}
