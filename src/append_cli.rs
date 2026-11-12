//! Post-execution CLI access to this terminal's last completed text recording.
use crate::{
    append::append_file,
    capture::{CAPTURE_LIMIT, CapturedRun},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Seek, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stream {
    Combined,
    Stdout,
    Stderr,
}
impl Stream {
    fn text(self, source: &CapturedRun) -> Result<(&str, bool)> {
        if self == Self::Combined {
            return Ok((&source.output, source.truncated));
        }
        let streams=source.streams.as_ref().context("This recording predates separate stream capture. Restart Tide and run the source command again")?;
        Ok(match self {
            Self::Stdout => (&streams.stdout, streams.stdout_truncated),
            Self::Stderr => (&streams.stderr, streams.stderr_truncated),
            Self::Combined => unreachable!(),
        })
    }
    fn label(self) -> &'static str {
        match self {
            Self::Combined => "stdout and stderr combined",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Pipe(String, Stream),
    AfterSuccess(String),
    File(PathBuf, Stream),
}
impl Action {
    pub fn parse(fragment: &str) -> Result<Self> {
        let fragment = fragment.trim();
        ensure!(
            !fragment.contains('\0'),
            "Append actions cannot contain NUL"
        );
        if let Some(command) = fragment.strip_prefix("&&") {
            ensure!(!command.trim().is_empty(), "Enter a command after &&");
            return Ok(Self::AfterSuccess(command.trim().into()));
        }
        let (stream, fragment) = if let Some(rest) = fragment.strip_prefix('1') {
            (Stream::Stdout, rest)
        } else if let Some(rest) = fragment.strip_prefix('2') {
            (Stream::Stderr, rest)
        } else {
            (Stream::Combined, fragment)
        };
        if let Some(command) = fragment.strip_prefix('|') {
            ensure!(
                !command.starts_with(['|', '&']),
                "Use | COMMAND, 1| COMMAND or 2| COMMAND"
            );
            ensure!(!command.trim().is_empty(), "Enter a command after |");
            return Ok(Self::Pipe(command.trim().into(), stream));
        }
        if let Some(path) = fragment.strip_prefix(">>") {
            return Ok(Self::File(literal_path(path.trim())?, stream));
        }
        bail!(
            "Use A_: '| COMMAND', '1| COMMAND', '2| COMMAND', '>> FILE', '1>> FILE', '2>> FILE', or '&& COMMAND'. 2>&1 merges streams; it does not name a file"
        )
    }
}
// Exactly one literal shell-style word. No expansions or command execution for filenames.
fn literal_path(text: &str) -> Result<PathBuf> {
    let mut result = String::new();
    let mut quote = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') | (Some('"'), '\\') => {
                result.push(chars.next().context("Incomplete filename escape")?);
            }
            (None, c) if c.is_whitespace() || "|&;<>".contains(c) => {
                bail!(
                    "Use one filename after >>; quote paths containing spaces or shell punctuation"
                )
            }
            _ => result.push(c),
        }
    }
    ensure!(quote.is_none(), "Unclosed filename quote");
    ensure!(!result.is_empty(), "Enter a file path after >>");
    Ok(result.into())
}

pub fn run(fragment: &str) -> Result<i32> {
    let action = Action::parse(fragment)?;
    let socket = std::env::var_os("TIDE_APPEND_SOCKET").context(
        "A_: needs an integrated Tide shell. Restart Tide after updating and run a command first",
    )?;
    let mut stream =
        UnixStream::connect(socket).context("This Tide capture session is no longer available")?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = Vec::new();
    (&mut stream)
        .take((CAPTURE_LIMIT * 18 + 131_072) as u64)
        .read_to_end(&mut bytes)?;
    let source: Option<CapturedRun> = serde_json::from_slice(&bytes)?;
    let source = source.context("No completed command is captured in this terminal yet")?;
    let code = source
        .exit_code
        .context("The source command has not finished")?;
    ensure!(
        code >= 0,
        "Source capture is unavailable: exit status is unknown or stream boundaries were lost"
    );
    if matches!(action, Action::AfterSuccess(_)) && code != 0 {
        eprintln!("A_: skipped follow-up; source exited with status {code}");
        return Ok(code);
    }
    let selected = match &action {
        Action::Pipe(_, stream) | Action::File(_, stream) => *stream,
        Action::AfterSuccess(_) => Stream::Combined,
    };
    let (output, truncated) = selected.text(&source)?;
    if !matches!(action, Action::AfterSuccess(_)) {
        eprintln!(
            "A_: using captured terminal text ({}{})",
            selected.label(),
            if truncated {
                "; only the retained tail is available"
            } else {
                ""
            }
        );
    }
    let pipe = matches!(action, Action::Pipe(..));
    match action {
        Action::File(path, _) => {
            let path = if path.is_absolute() {
                path
            } else {
                source.cwd.join(path)
            };
            let count = append_file(&path, output.as_bytes())
                .with_context(|| format!("Could not append to {}", path.display()))?;
            eprintln!("A_: appended {count} bytes to {}", path.display());
            Ok(0)
        }
        Action::Pipe(command, _) | Action::AfterSuccess(command) => {
            let mut child = Command::new(&source.shell);
            child.args(["-c", &command]).current_dir(&source.cwd);
            if pipe {
                // An unlinked regular file gives the entire shell a finite byte stream,
                // avoids pipe-buffer deadlocks, and needs no feeder process on Ctrl+C.
                let mut input = tempfile::tempfile()?;
                input.write_all(output.as_bytes())?;
                input.rewind()?;
                child.stdin(Stdio::from(input));
            }
            Err(child.exec()).context("Could not execute appended command")
        }
    }
}

/// Private per-session socket. Captured output stays in memory until explicitly consumed.
pub struct SnapshotServer {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    _directory: tempfile::TempDir,
}
impl SnapshotServer {
    pub fn new(completed: Arc<Mutex<Option<CapturedRun>>>) -> Result<Self> {
        Self::with_gate(completed, Arc::new(AtomicBool::new(true)))
    }
    pub fn with_gate(
        completed: Arc<Mutex<Option<CapturedRun>>>,
        ready: Arc<AtomicBool>,
    ) -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("tide-append-").tempdir()?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let path = directory.path().join("capture.sock");
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("tide-append".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let start = std::time::Instant::now();
                            while !ready.load(Ordering::Acquire)
                                && !stopping.load(Ordering::Acquire)
                                && start.elapsed() < Duration::from_secs(2)
                            {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            let snapshot = if ready.load(Ordering::Acquire) {
                                completed.lock().unwrap().clone()
                            } else {
                                None
                            };
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                            if let Ok(bytes) = serde_json::to_vec(&snapshot) {
                                let _ = stream.write_all(&bytes);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
            _directory: directory,
        })
    }
}
impl Drop for SnapshotServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
