use std::time::{Duration, Instant};
use tide_terminal::session::{Event, Launch, Options, Session};

fn until(mut predicate: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

#[test]
fn closing_interactive_shell_stops_hup_resistant_foreground_groups_and_reaps_shell() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("job.pid");
        let mut options = Options::shell(shell, dir.path().into());
        options.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        options.private = true;
        let session = Session::spawn(options).unwrap();
        let mut root = 0;
        assert!(
            until(|| {
                for event in session.events.try_iter() {
                    if let Event::Started { pid, .. } = event {
                        root = pid;
                    }
                }
                session.state.lock().unwrap().at_prompt
            }),
            "{shell}: shell did not reach prompt"
        );
        let script = format!(
            "import os,signal,time,pathlib; signal.signal(signal.SIGHUP,signal.SIG_IGN); pathlib.Path({:?}).write_text(str(os.getpid())); time.sleep(8)",
            pid_file.to_str().unwrap()
        );
        session
            .write(format!("python3 -u -c '{script}'\n").into_bytes())
            .unwrap();
        assert!(until(|| pid_file.exists()), "{shell}: child did not start");
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let start = Instant::now();
        drop(session);
        let closed = until(|| !std::path::Path::new(&format!("/proc/{pid}")).exists());
        if !closed {
            // This PID belongs to this bounded test fixture, not a desktop application.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(
            closed,
            "{shell}: foreground PID {pid} survived terminal close (or remained a zombie)"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(root > 1);
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(root as i32, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

#[test]
fn background_jobs_at_an_idle_prompt_are_detected_and_closed_in_each_shell() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("background.pid");
        let mut options = Options::shell(shell, dir.path().into());
        options.launch = Launch::Shell {
            path: shell.into(),
            clean: true,
        };
        options.private = true;
        let session = Session::spawn(options).unwrap();
        assert!(until(|| session.state.lock().unwrap().at_prompt));
        let script = format!(
            "import os,signal,time,pathlib; signal.signal(signal.SIGHUP,signal.SIG_IGN); pathlib.Path({:?}).write_text(str(os.getpid())); time.sleep(8)",
            pid_file.to_str().unwrap()
        );
        session
            .write(format!("python3 -u -c '{script}' &\n").into_bytes())
            .unwrap();
        assert!(until(
            || pid_file.exists() && session.state.lock().unwrap().at_prompt
        ));
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert!(
            session
                .active_processes()
                .iter()
                .any(|name| name.contains(&format!("PID {pid}"))),
            "{shell}: background process was missed"
        );
        // Stopped background jobs still belong to the terminal and must be
        // continued during shutdown so pending hangup signals can take effect.
        assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGSTOP) }, 0);
        assert!(
            session
                .active_processes()
                .iter()
                .any(|name| name.contains(&format!("PID {pid}")))
        );
        drop(session);
        assert!(
            until(|| !std::path::Path::new(&format!("/proc/{pid}")).exists()),
            "{shell}: background process survived or stayed zombie"
        );
    }
}

#[test]
fn close_after_leader_exit_cleans_remaining_session_but_preserves_another_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("orphan.pid");
    let script = format!(
        "import os,signal,time,pathlib\np=os.fork()\nif p:\n while not pathlib.Path({0:?}).exists(): time.sleep(.01)\n os._exit(0)\nsignal.signal(signal.SIGHUP,signal.SIG_IGN)\npathlib.Path({0:?}).write_text(str(os.getpid()))\ntime.sleep(8)",
        pid_file.to_str().unwrap()
    );
    let launch = |script: String| {
        let mut options = Options::shell("/bin/bash", dir.path().into());
        options.launch = Launch::Command(vec!["python3".into(), "-u".into(), "-c".into(), script]);
        options.private = true;
        Session::spawn(options).unwrap()
    };
    let unrelated = launch("import time;time.sleep(8)".into());
    let session = launch(script);
    assert!(until(|| !session.is_running()));
    let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    assert!(
        session
            .active_processes()
            .iter()
            .any(|name| name.contains(&format!("PID {pid}")))
    );
    drop(session);
    assert!(
        until(|| !std::path::Path::new(&format!("/proc/{pid}")).exists()),
        "orphan survived or stayed zombie"
    );
    assert!(unrelated.is_running());
    assert!(!unrelated.active_processes().is_empty());
}

#[test]
fn deliberately_detached_new_session_is_not_killed_on_terminal_close() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("detached.pid");
    let script = format!(
        "import os,signal,time,pathlib\np=os.fork()\nif p:\n while not pathlib.Path({0:?}).exists(): time.sleep(.01)\n os._exit(0)\nos.setsid()\nsignal.signal(signal.SIGHUP,signal.SIG_IGN)\npathlib.Path({0:?}).write_text(str(os.getpid()))\ntime.sleep(8)",
        pid_file.to_str().unwrap()
    );
    let mut options = Options::shell("/bin/bash", dir.path().into());
    options.launch = Launch::Command(vec!["python3".into(), "-u".into(), "-c".into(), script]);
    options.private = true;
    let session = Session::spawn(options).unwrap();
    assert!(until(|| !session.is_running()));
    let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    // Pin only this fixture for explicit cleanup, including after it is detached.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(fd >= 0);
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    assert!(session.active_processes().is_empty());
    drop(session);
    let survived = std::path::Path::new(&format!("/proc/{pid}")).exists();
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        );
    }
    assert!(survived, "deliberately detached application was killed");
    assert!(
        until(|| !std::path::Path::new(&format!("/proc/{pid}")).exists()),
        "detached fixture was not reaped by init"
    );
}
