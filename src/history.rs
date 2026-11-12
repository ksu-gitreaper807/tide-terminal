use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub command: String,
    pub cwd: String,
    pub shell: String,
    pub exit_code: i32,
    pub timestamp: i64,
}

pub struct History {
    db: Connection,
}
impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS history(id INTEGER PRIMARY KEY, command TEXT NOT NULL, cwd TEXT NOT NULL, shell TEXT NOT NULL, exit_code INTEGER NOT NULL, timestamp INTEGER NOT NULL DEFAULT(unixepoch())); CREATE INDEX IF NOT EXISTS history_recent ON history(timestamp DESC);")?;
        Ok(Self { db })
    }
    pub fn record(&self, command: &str, cwd: &str, shell: &str, exit_code: i32) -> Result<()> {
        if command.trim().is_empty() || command.starts_with(' ') || command.len() > 16_384 {
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO history(command,cwd,shell,exit_code) VALUES(?1,?2,?3,?4)",
            params![command, cwd, shell, exit_code],
        )?;
        Ok(())
    }
    pub fn search(&self, query: &str, cwd: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt=self.db.prepare("SELECT command,cwd,shell,exit_code,timestamp FROM (SELECT *,row_number() OVER(PARTITION BY command ORDER BY timestamp DESC,id DESC) AS rank FROM history) WHERE rank=1 AND instr(lower(command),lower(?1))>0 ORDER BY (cwd=?2) DESC,(substr(command,1,length(?1))=?1) DESC,timestamp DESC,id DESC LIMIT ?3")?;
        let rows = stmt.query_map(params![query, cwd, limit.min(500) as i64], |r| {
            Ok(HistoryEntry {
                command: r.get(0)?,
                cwd: r.get(1)?,
                shell: r.get(2)?,
                exit_code: r.get(3)?,
                timestamp: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn suggest(&self, prefix: &str, cwd: &str) -> Result<Option<String>> {
        if prefix.trim().is_empty() || prefix.starts_with(' ') {
            return Ok(None);
        }
        Ok(self
            .search(prefix, cwd, 30)?
            .into_iter()
            .find(|r| r.command.starts_with(prefix) && r.command != prefix)
            .map(|r| r.command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_scoped_ranked_and_private_entries_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(&dir.path().join("h.db")).unwrap();
        h.record("git status", "/a", "bash", 0).unwrap();
        h.record("git stash", "/b", "zsh", 0).unwrap();
        h.record(" secret password", "/a", "fish", 0).unwrap();
        h.record("git status", "/a", "fish", 1).unwrap();
        let rows = h.search("git", "/a", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].command, "git status");
        assert_eq!(rows[0].shell, "fish");
        assert_eq!(
            h.suggest("git st", "/a").unwrap(),
            Some("git status".into())
        );
        assert!(h.search("secret", "", 10).unwrap().is_empty());
    }
}
