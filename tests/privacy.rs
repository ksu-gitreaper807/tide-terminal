use std::{
    path::Path,
    time::{Duration, Instant},
};
use tide_terminal::session::{Event, Options, Session};

fn wait(session: &Session, predicate: impl Fn(&Event) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Ok(event) = session.events.recv_timeout(Duration::from_millis(50))
            && predicate(&event)
        {
            return;
        }
    }
    panic!("Private shell did not reach expected state");
}
fn assert_private_files(root: &Path) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_private_files(&path);
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            assert!(
                !text.contains("PRIVATE_NEVER_PERSIST"),
                "Private command persisted at {}",
                path.display()
            );
        }
    }
}
#[test]
fn shell_rc_history_overrides_do_not_defeat_private_sessions() {
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".bashrc"), "HISTFILE=$HOME/leak_history\n").unwrap();
        std::fs::write(
            dir.path().join(".zshrc"),
            "HISTFILE=$HOME/leak_history\nSAVEHIST=100\nHISTSIZE=100\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("config/fish")).unwrap();
        std::fs::write(
            dir.path().join("config/fish/config.fish"),
            "set -g fish_history leak\n",
        )
        .unwrap();
        let mut o = Options::shell(shell, dir.path().to_path_buf());
        o.private = true;
        o.history = Some(dir.path().join("tide.db"));
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
        wait(&s, |e| matches!(e, Event::Prompt));
        s.write(b"printf PRIVATE_NEVER_PERSIST\n".to_vec()).unwrap();
        wait(&s, |e| matches!(e, Event::CommandEnded { .. }));
        s.write(b"exit\n".to_vec()).unwrap();
        wait(&s, |e| matches!(e, Event::Exited { .. }));
        drop(s);
        assert!(!dir.path().join("tide.db").exists());
        assert_private_files(dir.path());
    }
}
