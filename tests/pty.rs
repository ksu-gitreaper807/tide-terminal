use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tide_terminal::{
    history::History,
    rules::RuleSpec,
    session::{Event, Launch, Options, Session},
};

fn command(script: &str, rules: RuleSpec) -> Options {
    let mut o = Options::shell("/bin/bash", std::env::temp_dir());
    o.launch = Launch::Command(vec![
        "/usr/bin/python3".into(),
        "-u".into(),
        "-c".into(),
        script.into(),
    ]);
    o.rules = rules;
    o.private = true;
    o
}
fn wait(session: &Session, timeout: Duration, predicate: impl Fn(&Event) -> bool) -> Vec<Event> {
    let start = Instant::now();
    let mut events = vec![];
    while start.elapsed() < timeout {
        if let Ok(e) = session.events.recv_timeout(Duration::from_millis(50)) {
            let done = predicate(&e);
            events.push(e);
            if done {
                return events;
            }
        }
    }
    panic!("Timed out waiting for event; received {events:?}");
}
fn finish(s: &Session) -> Vec<Event> {
    wait(s, Duration::from_secs(8), |e| {
        matches!(e, Event::Exited { .. })
    })
}

#[test]
fn timer_colon_wrapper_works_in_all_integrated_shells() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let directory = tempfile::tempdir().unwrap();
        let mut options = Options::shell(shell, directory.path().to_path_buf());
        options.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        options.private = true;
        // Session tests normally expose the test runner as TIDE_EXE; use the actual CLI here.
        options.env.push((
            "TIDE_EXE".into(),
            env!("CARGO_BIN_EXE_tide-terminal").into(),
        ));
        let session = Session::spawn(options).unwrap();
        let prompt = |event: &Event| matches!(event, Event::Prompt);
        wait(&session, Duration::from_secs(5), prompt);
        session
            .write(b"T_: 'sleep 20' : -s 0.25 -grace 100ms\n".to_vec())
            .unwrap();
        let events = wait(&session, Duration::from_secs(5), |event| {
            matches!(event, Event::CommandEnded { .. })
        });
        let output: String = events
            .iter()
            .filter_map(|event| match event {
                Event::Output { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            output.contains("Runtime limit reached"),
            "{shell}: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::CommandEnded { code, .. } if *code != 0)),
            "{shell}: {events:?}"
        );
        session.write(b"exit\n".to_vec()).unwrap();
        finish(&session);
    }
}

#[test]
fn tty_controls_interrupt_suspend_resume_quit_and_end_input_in_each_shell() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let directory = tempfile::tempdir().unwrap();
        let mut options = Options::shell(shell, directory.path().to_path_buf());
        options.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        options.private = true;
        options.suggestions = true;
        let session = Session::spawn(options).unwrap();
        let prompt = |event: &Event| matches!(event, Event::Prompt);
        wait(&session, Duration::from_secs(5), prompt);

        session.write(b"python3 -u -c 'import time; [print(\"JOB_TICK\",flush=True) or time.sleep(.05) for _ in range(600)]'\n".to_vec()).unwrap();
        wait(
            &session,
            Duration::from_secs(5),
            |event| matches!(event, Event::Output { text } if text.contains("JOB_TICK\r\n")),
        );
        session.write(vec![26]).unwrap(); // Ctrl+Z: the shell must regain control.
        wait(&session, Duration::from_secs(5), prompt);
        session.write(b"fg\n".to_vec()).unwrap();
        wait(
            &session,
            Duration::from_secs(5),
            |event| matches!(event, Event::Output { text } if text.contains("JOB_TICK\r\n")),
        );
        session.write(vec![3]).unwrap(); // Ctrl+C after resuming the job.
        let events = wait(&session, Duration::from_secs(5), prompt);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Output { text } if text.contains("KeyboardInterrupt"))),
            "{shell}: {events:?}"
        );

        session.write(b"python3 -u -c 'import sys; print(\"EOF_READY\",flush=True); sys.stdin.read(); print(\"EOF_SEEN\")'\n".to_vec()).unwrap();
        wait(
            &session,
            Duration::from_secs(5),
            |event| matches!(event, Event::Output { text } if text.contains("EOF_READY\r\n")),
        );
        session.write(vec![4]).unwrap(); // Ctrl+D: EOF, not a process signal.
        let events = wait(&session, Duration::from_secs(5), prompt);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Output { text } if text.contains("EOF_SEEN"))),
            "{shell}: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::CommandEnded { code: 0, .. }))
        );

        session.write(b"python3 -u -c 'import resource,time; resource.setrlimit(resource.RLIMIT_CORE,(0,0)); print(\"QUIT_READY\",flush=True); time.sleep(30)'\n".to_vec()).unwrap();
        wait(
            &session,
            Duration::from_secs(5),
            |event| matches!(event, Event::Output { text } if text.contains("QUIT_READY\r\n")),
        );
        session.write(vec![28]).unwrap(); // Ctrl+backslash: SIGQUIT via the tty.
        let events = wait(&session, Duration::from_secs(5), prompt);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::CommandEnded { code: 131, .. })),
            "{shell}: {events:?}"
        );
        session.write(b"exit\n".to_vec()).unwrap();
        finish(&session);
    }
}

#[test]
fn fingerprint_prompt_alerts_without_keyboard_input_and_handles_password_fallback() {
    for fallback in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let scanned = directory.path().join("scan-completed");
        let marker = serde_json::to_string(&scanned.to_string_lossy()).unwrap();
        let script = format!(
            "import sys,time,pathlib,getpass\nsys.stdout.write('\\x1b[33mPlace your fi');sys.stdout.flush()\ntime.sleep(.05)\nprint('nger on the fingerprint reader\\x1b[0m',flush=True)\nwhile not pathlib.Path({marker}).exists():\n time.sleep(.02)\n{}\nprint('Authentication complete',flush=True)\ntime.sleep(.5)",
            if fallback {
                "getpass.getpass('Password: ')"
            } else {
                "pass"
            }
        );
        let session = Session::spawn(command(&script, RuleSpec::default())).unwrap();
        let events = wait(
            &session,
            Duration::from_secs(4),
            |event| matches!(event, Event::Attention { message, .. } if message.contains("fingerprint")),
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Attention { .. }))
                .count(),
            1
        );
        std::thread::sleep(Duration::from_millis(450));
        assert!(
            session
                .state
                .lock()
                .unwrap()
                .attention
                .as_ref()
                .is_some_and(|message| message.contains("fingerprint"))
        );
        assert!(
            !session
                .events
                .try_iter()
                .any(|event| matches!(event, Event::Attention { .. }))
        );

        // A scanner completes authentication outside the keyboard/PTY input path.
        std::fs::write(&scanned, b"done").unwrap();
        if fallback {
            wait(
                &session,
                Duration::from_secs(4),
                |event| matches!(event, Event::Attention { message, .. } if message.contains("password")),
            );
            session.write(b"test-only-password\n".to_vec()).unwrap();
        }
        wait(&session, Duration::from_secs(4), |event| {
            matches!(event, Event::AttentionCleared)
        });
        assert!(session.state.lock().unwrap().attention.is_none());
        assert!(
            finish(&session)
                .iter()
                .any(|event| matches!(event, Event::Exited { code: 0, .. }))
        );
    }
}

#[test]
fn actual_password_prompt_alerts_once_and_accepts_input_without_recording_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("history.db");
    let mut o = command(
        "import getpass; x=getpass.getpass('Password: '); print('ACCEPTED' if len(x) == 11 else 'BAD')",
        RuleSpec::default(),
    );
    o.private = false;
    o.history = Some(db.clone());
    let s = Session::spawn(o).unwrap();
    let events = wait(&s, Duration::from_secs(4), |e| {
        matches!(e, Event::Attention { .. })
    });
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Attention { .. }))
            .count(),
        1
    );
    assert!(!s.state.lock().unwrap().at_prompt);
    std::thread::sleep(Duration::from_millis(450));
    assert!(
        !s.events
            .try_iter()
            .any(|e| matches!(e, Event::Attention { .. }))
    );
    s.write(b"secret-test\n".to_vec()).unwrap();
    let events = finish(&s);
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("ACCEPTED")))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("secret-test")))
    );
    assert!(
        History::open(&db)
            .unwrap()
            .search("secret-test", "", 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn timer_interrupts_and_force_escalates_an_owned_group() {
    let script = "import signal,time; signal.signal(signal.SIGINT, signal.SIG_IGN); print('READY'); time.sleep(30)";
    let s = Session::spawn(command(
        script,
        RuleSpec {
            timeout: Some(Duration::from_millis(350)),
            grace: Duration::from_millis(120),
            force: true,
            ..Default::default()
        },
    ))
    .unwrap();
    let start = Instant::now();
    let events = finish(&s);
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::RuleTriggered{reason,..}if reason.contains("Runtime")))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Escalated { .. })));
    assert!(s.state.lock().unwrap().exit_code.is_some_and(|c| c != 0));
}

#[test]
fn split_colored_marker_stops_program_before_late_side_effect() {
    let dir = tempfile::tempdir().unwrap();
    let late = dir.path().join("late");
    let script = format!(
        "import time,pathlib; print('\\x1b[32mTRAIN',end='',flush=True); time.sleep(.08); print('ING_COMPLETE\\x1b[0m',flush=True); time.sleep(3); pathlib.Path({:?}).touch()",
        late.to_string_lossy()
    );
    let s = Session::spawn(command(
        &script,
        RuleSpec {
            literal: Some("TRAINING_COMPLETE".into()),
            grace: Duration::from_millis(100),
            force: true,
            ..Default::default()
        },
    ))
    .unwrap();
    let events = finish(&s);
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::RuleTriggered{reason,..}if reason.contains("marker")))
    );
    assert!(!late.exists());
}

#[test]
fn regex_idle_and_confirmation_are_real_pty_behaviors() {
    for (script, rules, reason) in [
        (
            "import time; print('FATAL: lost connection'); time.sleep(20)",
            RuleSpec {
                pattern: Some("FATAL:.*connection".into()),
                grace: Duration::from_millis(100),
                force: true,
                ..Default::default()
            },
            "pattern",
        ),
        (
            "import time; print('quiet now'); time.sleep(20)",
            RuleSpec {
                idle: Some(Duration::from_millis(250)),
                grace: Duration::from_millis(100),
                force: true,
                ..Default::default()
            },
            "inactivity",
        ),
    ] {
        let s = Session::spawn(command(script, rules)).unwrap();
        assert!(
            finish(&s)
                .iter()
                .any(|e| matches!(e,Event::RuleTriggered{reason:r,..}if r.contains(reason)))
        );
    }
    let s = Session::spawn(command(
        "x=input('Continue? [y/N] '); print('ANSWER='+x)",
        RuleSpec::default(),
    ))
    .unwrap();
    assert!(
        wait(&s, Duration::from_secs(4), |e| matches!(
            e,
            Event::Attention { .. }
        ))
        .iter()
        .any(|e| matches!(e,Event::Attention{message,..}if message.contains("confirmation")))
    );
    s.write(b"y\n".to_vec()).unwrap();
    assert!(
        finish(&s)
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("ANSWER=y")))
    );
}

#[test]
fn editor_alternate_screen_does_not_trigger_prompt_heuristics() {
    let s=Session::spawn(command("import time; print('\\x1b[?1049hPassword:',end='',flush=True); time.sleep(.7); print('\\x1b[?1049l')",RuleSpec::default())).unwrap();
    assert!(
        !finish(&s)
            .iter()
            .any(|e| matches!(e, Event::Attention { .. }))
    );
}

#[test]
fn finished_timer_cannot_affect_a_different_run() {
    let first = Session::spawn(command(
        "print('fast')",
        RuleSpec {
            timeout: Some(Duration::from_millis(300)),
            force: true,
            ..Default::default()
        },
    ))
    .unwrap();
    finish(&first);
    let next = Session::spawn(command(
        "import time; time.sleep(.6); print('SURVIVED')",
        RuleSpec::default(),
    ))
    .unwrap();
    let events = finish(&next);
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("SURVIVED")))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Stopping { .. } | Event::RuleTriggered { .. }))
    );
}

#[test]
fn all_three_shells_preserve_language_report_boundaries_and_share_history() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("history.db");
    for (shell, cmd) in [
        ("/bin/bash", "v=42; printf 'BASH_%s\\n' \"$v\""),
        ("/bin/zsh", "typeset v=42; printf 'ZSH_%s\\n' \"$v\""),
        ("/usr/bin/fish", "set v 42; printf 'FISH_%s\\n' $v"),
    ] {
        let mut o = Options::shell(shell, PathBuf::from("/tmp"));
        o.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        o.history = Some(db.clone());
        o.env.push((
            "HISTFILE".into(),
            dir.path()
                .join("shell_history")
                .to_string_lossy()
                .into_owned(),
        ));
        o.env.push((
            "XDG_DATA_HOME".into(),
            dir.path().join("data").to_string_lossy().into_owned(),
        ));
        o.env.push((
            "XDG_CONFIG_HOME".into(),
            dir.path().join("config").to_string_lossy().into_owned(),
        ));
        let s = Session::spawn(o).unwrap();
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        s.write(format!("{cmd}\n").into_bytes()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::CommandStarted{command,..}if command==cmd)),
            "{shell}: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CommandEnded { code: 0, .. })),
            "{shell}: {events:?}"
        );
        s.write(b"exit\n".to_vec()).unwrap();
        finish(&s);
    }
    let rows = History::open(&db)
        .unwrap()
        .search("printf", "/tmp", 100)
        .unwrap();
    assert!(rows.iter().any(|e| e.shell == "bash"));
    assert!(rows.iter().any(|e| e.shell == "zsh"));
    assert!(rows.iter().any(|e| e.shell == "fish"));
}

#[test]
fn enhanced_suggestions_render_and_accept_in_bash_zsh_and_fish() {
    let root = tempfile::tempdir().unwrap();
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = root.path().join(shell.replace('/', "_"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tide_completion_probe file.txt"),
            "TAB_COMPLETION_OK\n",
        )
        .unwrap();
        let mut o = Options::shell(shell, dir.clone());
        o.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        o.suggestions = true;
        o.env = vec![
            (
                "HISTFILE".into(),
                dir.join("history").to_string_lossy().into_owned(),
            ),
            (
                "XDG_DATA_HOME".into(),
                dir.join("data").to_string_lossy().into_owned(),
            ),
            (
                "XDG_CACHE_HOME".into(),
                dir.join("cache").to_string_lossy().into_owned(),
            ),
            (
                "XDG_CONFIG_HOME".into(),
                dir.join("config").to_string_lossy().into_owned(),
            ),
        ];
        let s = Session::spawn(o).unwrap();
        wait(&s, Duration::from_secs(12), |e| matches!(e, Event::Prompt));
        s.write(b"printf 'TIDE_SUGGESTION_PROBE_12345\\n'\n".to_vec())
            .unwrap();
        let events = wait(&s, Duration::from_secs(8), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(events.iter().any(|e|matches!(e,Event::CommandStarted{command,..}if command.contains("TIDE_SUGGESTION_PROBE"))),"{shell}: {events:?}");
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        // Allow the shell to finish redrawing before entering a new partial line.
        std::thread::sleep(Duration::from_millis(150));
        s.write(b"printf 'TIDE_SUGGEST".to_vec()).unwrap();
        let start = Instant::now();
        let mut ghost = String::new();
        while start.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(50));
            let v = s.snapshot();
            ghost = v
                .cells
                .iter()
                .filter(|c| c.row as i32 == v.cursor.0 && c.col >= v.cursor.1)
                .map(|c| c.text.as_str())
                .collect();
            if ghost.contains("ION_PROBE_12345") {
                break;
            }
        }
        assert!(
            ghost.contains("ION_PROBE_12345"),
            "{shell}: no inline suggestion after cursor: {ghost:?}"
        );
        s.write(b"\x1b[C\n".to_vec()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(events.iter().any(|e|matches!(e,Event::CommandStarted{command,..}if command.contains("TIDE_SUGGESTION_PROBE_12345"))),"{shell}: suggestion not accepted: {events:?}");
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        std::thread::sleep(Duration::from_millis(150));
        s.write(b"garbage_to_replace".to_vec()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let replacement = "printf '置換_BUFFER_OK\\n'";
        s.insert_command(replacement).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !s.events
                .try_iter()
                .any(|e| matches!(e, Event::CommandStarted { .. })),
            "History insertion must not execute"
        );
        s.write(b"\n".to_vec()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::CommandStarted{command,..}if command==replacement)),
            "{shell}: replacement failed: {events:?}"
        );
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        std::thread::sleep(Duration::from_millis(100));
        s.write(b"cat tide_completion_p\t".to_vec()).unwrap();
        // Completion is asynchronous (especially ble.sh's first uncached lookup).
        // Observe the edited buffer instead of racing it with a fixed 250ms Enter.
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let text: String = s
                .snapshot()
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect();
            if text.contains("tide_completion_probe") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{shell}: completion did not update the editor: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        s.write(b"\n".to_vec()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::Output{text}if text.contains("TAB_COMPLETION_OK"))),
            "{shell}: file completion failed: {events:?}"
        );
        s.write(b"exit\n".to_vec()).unwrap();
        finish(&s);
    }
}

#[test]
fn very_long_output_lines_still_detect_trailing_markers() {
    let s=Session::spawn(command("import sys,time;sys.stdout.write('x'*100000+'LONG_LINE_DONE');sys.stdout.flush();time.sleep(20)",RuleSpec{literal:Some("LONG_LINE_DONE".into()),grace:Duration::from_millis(100),force:true,..Default::default()})).unwrap();
    assert!(
        finish(&s)
            .iter()
            .any(|e| matches!(e,Event::RuleTriggered{reason,..}if reason.contains("marker")))
    );
}

#[test]
fn blocked_stdin_does_not_block_runtime_limits() {
    let s=Session::spawn(command("import tty,sys,time,signal; tty.setraw(sys.stdin.fileno());signal.signal(signal.SIGINT,signal.SIG_IGN);print('READY',flush=True);time.sleep(30)",RuleSpec{timeout:Some(Duration::from_millis(500)),grace:Duration::from_millis(100),force:true,..Default::default()})).unwrap();
    wait(
        &s,
        Duration::from_secs(3),
        |e| matches!(e,Event::Output{text}if text.contains("READY")),
    );
    s.write(vec![b'x'; 1_048_576]).unwrap();
    let start = Instant::now();
    let events = finish(&s);
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(events.iter().any(|e| matches!(e, Event::Escalated { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Exited {
            signal: Some(_),
            ..
        }
    )));
}

#[test]
fn user_startup_aliases_hooks_and_failure_status_survive_integration() {
    for (shell, rc) in [
        (
            "/bin/bash",
            "alias tide_probe='printf ORIGINAL_ALIAS_OK'\nPROMPT_COMMAND='printf ORIGINAL_HOOK_%s \"$?\"'\n",
        ),
        (
            "/bin/zsh",
            "alias tide_probe='printf ORIGINAL_ALIAS_OK'\nprecmd() { printf ORIGINAL_HOOK_%s \"$?\"; }\n",
        ),
        (
            "/usr/bin/fish",
            "alias tide_probe 'printf ORIGINAL_ALIAS_OK'\nfunction original_hook --on-event fish_prompt\n printf ORIGINAL_HOOK\nend\n",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".bashrc"), rc).unwrap();
        std::fs::write(dir.path().join(".zshrc"), rc).unwrap();
        std::fs::create_dir_all(dir.path().join("config/fish")).unwrap();
        std::fs::write(dir.path().join("config/fish/config.fish"), rc).unwrap();
        let mut o = Options::shell(shell, dir.path().to_path_buf());
        o.env = vec![
            ("HOME".into(), dir.path().to_string_lossy().into_owned()),
            (
                "TIDE_ORIGINAL_ZDOTDIR".into(),
                dir.path().to_string_lossy().into_owned(),
            ),
            (
                "XDG_CONFIG_HOME".into(),
                dir.path().join("config").to_string_lossy().into_owned(),
            ),
            (
                "XDG_DATA_HOME".into(),
                dir.path().join("data").to_string_lossy().into_owned(),
            ),
        ];
        let s = Session::spawn(o).unwrap();
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        s.write(b"tide_probe\n".to_vec()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Event::Output{text}if text.contains("ORIGINAL_ALIAS_OK"))),
            "{shell}: {events:?}"
        );
        wait(&s, Duration::from_secs(5), |e| matches!(e, Event::Prompt));
        s.write(b"false\n".to_vec()).unwrap();
        let events = wait(&s, Duration::from_secs(5), |e| {
            matches!(e, Event::CommandEnded { .. })
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CommandEnded { code: 1, .. })),
            "{shell}: failure status lost: {events:?}"
        );
        s.write(b"exit\n".to_vec()).unwrap();
        finish(&s);
    }
}

#[test]
fn force_stop_reaches_normal_descendants_but_leaves_unrelated_jobs_alone() {
    let dir = tempfile::tempdir().unwrap();
    let heartbeat = dir.path().join("heartbeat");
    let script = format!(
        "import os,signal,time; signal.signal(signal.SIGINT,signal.SIG_IGN); p=os.fork();\nwhile True:\n if p==0:\n  with open({:?},'a') as f: f.write('x'); f.flush()\n time.sleep(.03)",
        heartbeat.to_string_lossy()
    );
    let unrelated = Session::spawn(command(
        "import time;time.sleep(1.1);print('UNRELATED_OK')",
        RuleSpec::default(),
    ))
    .unwrap();
    let s = Session::spawn(command(
        &script,
        RuleSpec {
            timeout: Some(Duration::from_millis(350)),
            grace: Duration::from_millis(80),
            force: true,
            ..Default::default()
        },
    ))
    .unwrap();
    finish(&s);
    let size = std::fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(std::fs::metadata(&heartbeat).unwrap().len(), size);
    assert!(
        finish(&unrelated)
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("UNRELATED_OK")))
    );
}

#[test]
fn close_can_finish_a_non_forcing_stop_and_cooperative_events_work() {
    let s=Session::spawn(command("import signal,time; signal.signal(signal.SIGINT,signal.SIG_IGN); print('\\x1b]777;tide;input;approval\\a',flush=True);time.sleep(30)",RuleSpec{timeout:Some(Duration::from_millis(500)),force:false,..Default::default()})).unwrap();
    assert!(
        wait(&s, Duration::from_secs(3), |e| matches!(
            e,
            Event::Attention { reported: true, .. }
        ))
        .iter()
        .any(|e| matches!(e, Event::Attention { reported: true, .. }))
    );
    wait(&s, Duration::from_secs(3), |e| {
        matches!(e, Event::Stopping { .. })
    });
    let start = Instant::now();
    drop(s);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn resize_reaches_actual_tty_and_private_runs_leave_no_database() {
    let dir = tempfile::tempdir().unwrap();
    let mut o = command(
        "import os,time; time.sleep(.2); s=os.get_terminal_size(); print(f'SIZE={s.columns}x{s.lines}')",
        RuleSpec::default(),
    );
    o.history = Some(dir.path().join("should-not-exist.db"));
    let s = Session::spawn(o).unwrap();
    s.resize(42, 132);
    assert!(
        finish(&s)
            .iter()
            .any(|e| matches!(e,Event::Output{text}if text.contains("SIZE=132x42")))
    );
    assert!(!dir.path().join("should-not-exist.db").exists());
}
