use tide_terminal::history::History;
#[path = "support/deadline.rs"]
mod deadline;

#[test]
fn bundled_sqlite_includes_wal_close_open_deadlock_fixes() {
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "SQLite {} predates the WAL mutex-order and reset fixes",
        rusqlite::version()
    );
}

#[test]
fn opening_history_while_the_last_connection_closes_stays_responsive() {
    use std::sync::{Arc, Barrier};
    let deadline = deadline::Deadline::new(
        "history_close_open_race",
        std::time::Duration::from_secs(30),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let original = History::open(&path).unwrap();
    original
        .record("retained command", "/original", "bash", 17)
        .unwrap();
    drop(original);
    let phase = Arc::new(Barrier::new(2));
    let peer = phase.clone();
    let peer_path = path.clone();
    deadline.stage("opening history concurrently with last-connection close");
    let closer = std::thread::spawn(move || {
        for _ in 0..256 {
            let history = History::open(&peer_path).unwrap();
            assert_eq!(history.search("retained", "", 10).unwrap().len(), 1);
            peer.wait();
            drop(history);
            peer.wait();
        }
    });
    for _ in 0..256 {
        phase.wait();
        let history = History::open(&path).unwrap();
        let entries = history.search("retained", "", 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cwd, "/original");
        assert_eq!(entries[0].exit_code, 17);
        drop(history);
        phase.wait();
    }
    closer.join().unwrap();
}

fn history() -> (tempfile::TempDir, History) {
    let dir = tempfile::tempdir().unwrap();
    let history = History::open(&dir.path().join("nested/history.sqlite3")).unwrap();
    (dir, history)
}
#[test]
fn blank_private_and_oversized_commands_are_excluded() {
    let (_dir, h) = history();
    for cmd in [
        "".into(),
        "   ".into(),
        "\t\n".into(),
        " secret".into(),
        "x".repeat(16385),
    ] {
        h.record(&cmd, "/a", "bash", 0).unwrap();
    }
    assert!(h.search("", "", 500).unwrap().is_empty());
}
#[test]
fn maximum_length_command_is_retained() {
    let (_dir, h) = history();
    let cmd = "x".repeat(16384);
    h.record(&cmd, "/a", "bash", 0).unwrap();
    assert_eq!(h.search("x", "/a", 1).unwrap()[0].command, cmd);
}
#[test]
fn sql_metacharacters_are_literal_search_text() {
    let (_dir, h) = history();
    h.record("printf '%_'; echo \"' OR 1=1 --\"", "/a", "bash", 0)
        .unwrap();
    h.record("unrelated", "/a", "bash", 0).unwrap();
    for query in ["%_", "' OR 1=1 --", "PRINTF"] {
        assert_eq!(h.search(query, "/a", 10).unwrap().len(), 1);
    }
    assert!(h.search("missing%", "/a", 10).unwrap().is_empty());
}
#[test]
fn newest_duplicate_preserves_its_metadata() {
    let (_dir, h) = history();
    h.record("same", "/a", "bash", 0).unwrap();
    h.record("same", "/b", "fish", 42).unwrap();
    let rows = h.search("same", "/a", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (&*rows[0].cwd, &*rows[0].shell, rows[0].exit_code),
        ("/b", "fish", 42)
    );
}
#[test]
fn directory_scope_ranks_before_recency() {
    let (_dir, h) = history();
    h.record("git status", "/a", "bash", 0).unwrap();
    h.record("git stash", "/b", "fish", 0).unwrap();
    assert_eq!(h.search("git", "/a", 10).unwrap()[0].command, "git status");
}
#[test]
fn suggestions_require_a_prefix_and_do_not_repeat_complete_input() {
    let (_dir, h) = history();
    h.record("git status", "/a", "bash", 0).unwrap();
    for prefix in ["", " ", " git", "status", "git status"] {
        assert!(h.suggest(prefix, "/a").unwrap().is_none());
    }
    assert_eq!(
        h.suggest("git st", "/a").unwrap().as_deref(),
        Some("git status")
    );
}
#[test]
fn unicode_and_multiline_commands_round_trip() {
    let (_dir, h) = history();
    let cmd = "printf '日本🙂'\n# λ";
    h.record(cmd, "/tmp/日 fixture", "fish", 17).unwrap();
    let rows = h.search("日本", "", 10).unwrap();
    assert_eq!(rows[0].command, cmd);
    assert_eq!(rows[0].exit_code, 17);
}
#[test]
fn limits_are_zero_safe_and_capped() {
    let (_dir, h) = history();
    for i in 0..510 {
        h.record(&format!("cmd {i}"), "/a", "bash", 0).unwrap();
    }
    assert_eq!(h.search("", "", 0).unwrap().len(), 0);
    assert_eq!(h.search("", "", 3).unwrap().len(), 3);
    assert_eq!(h.search("", "", usize::MAX).unwrap().len(), 500);
}
#[test]
fn reopening_keeps_history_and_schema_creation_is_idempotent() {
    let (dir, h) = history();
    h.record("persistent", "/a", "zsh", 23).unwrap();
    drop(h);
    let h = History::open(&dir.path().join("nested/history.sqlite3")).unwrap();
    assert_eq!(h.search("persistent", "", 1).unwrap()[0].exit_code, 23);
}
#[test]
fn independent_connections_can_write_concurrently() {
    let (dir, h) = history();
    let path = dir.path().join("nested/history.sqlite3");
    let handles: Vec<_> = (0..4)
        .map(|writer| {
            let path = path.clone();
            std::thread::spawn(move || {
                let h = History::open(&path).unwrap();
                for i in 0..20 {
                    h.record(&format!("writer {writer} command {i}"), "/a", "bash", 0)
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(h.search("writer", "", 500).unwrap().len(), 80);
}
