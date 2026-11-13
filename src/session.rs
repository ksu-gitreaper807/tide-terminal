use crate::{
    capture::Recording,
    history::History,
    observer::{Marker, Observer, ProtocolGuard},
    platform,
    rules::{RuleSpec, Rules},
    split_capture::{Channel, SplitCapture},
    split_pty::{self, ReadEvent},
    terminal::{self, Core},
};
use alacritty_terminal::{event::Event as CoreEvent, vte::ansi};
use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, Sender, bounded, select};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
type Waker = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub enum Launch {
    Shell { path: String, clean: bool },
    Command(Vec<String>),
    ShellCommand { shell: String, command: String },
}
#[derive(Clone, Debug)]
pub struct Options {
    /// Captured output supplied as a finite stdin stream for a shell-command run.
    pub stdin: Option<Vec<u8>>,
    pub launch: Launch,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    pub rules: RuleSpec,
    pub history: Option<PathBuf>,
    pub private: bool,
    pub notifications: bool,
    pub suggestions: bool,
    pub env: Vec<(String, String)>,
}
impl Options {
    pub fn shell(shell: &str, cwd: PathBuf) -> Self {
        Self {
            stdin: None,
            launch: Launch::Shell {
                path: shell.into(),
                clean: false,
            },
            cwd,
            rows: 30,
            cols: 100,
            rules: RuleSpec::default(),
            history: None,
            private: false,
            notifications: false,
            suggestions: false,
            env: vec![],
        }
    }
    pub fn managed(&self) -> bool {
        !matches!(self.launch, Launch::Shell { .. })
    }
    pub fn label(&self) -> String {
        match &self.launch {
            Launch::Shell { path, .. } => PathBuf::from(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Launch::Command(argv) => argv.join(" "),
            Launch::ShellCommand { command, .. } => command.clone(),
        }
    }
    pub fn shell_name(&self) -> String {
        match &self.launch {
            Launch::Shell { path, .. } | Launch::ShellCommand { shell: path, .. } => {
                PathBuf::from(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }
            Launch::Command(_) => "exec".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Started {
        run_id: u64,
        pid: u32,
        command: String,
        managed: bool,
    },
    Output {
        text: String,
    },
    Prompt,
    CommandStarted {
        run_id: u64,
        command: String,
    },
    CommandEnded {
        run_id: u64,
        code: i32,
    },
    Attention {
        run_id: u64,
        message: String,
        reported: bool,
    },
    AttentionCleared,
    RuleTriggered {
        run_id: u64,
        reason: String,
    },
    Stopping {
        run_id: u64,
        reason: String,
    },
    Escalated {
        run_id: u64,
    },
    Exited {
        run_id: u64,
        code: u32,
        signal: Option<String>,
        reason: Option<String>,
    },
    Error {
        message: String,
    },
}

pub struct State {
    pub recording: Option<Recording>,
    pub terminal: Core,
    pub title: String,
    pub cwd: String,
    pub command: String,
    /// Retained after completion, independently of the bounded notification queue.
    pub last_command: String,
    pub run_id: u64,
    pub running: bool,
    pub at_prompt: bool,
    pub managed: bool,
    pub attention: Option<String>,
    pub stop_reason: Option<String>,
    pub exit_code: Option<u32>,
    pub exit_signal: Option<String>,
    pub elapsed: Duration,
    pub changed: u64,
}
enum Control {
    Input(Vec<u8>),
    Resize(u16, u16),
    Stop,
    Close,
}
pub struct Session {
    pub id: u64,
    pub state: Arc<Mutex<State>>,
    pub events: Receiver<Event>,
    tx: Sender<Control>,
    worker: Option<std::thread::JoinHandle<()>>,
    wake: Waker,
    selection: Option<PathBuf>,
    processes: Arc<Mutex<crate::processes::SessionProcesses>>,
}

fn setup_command(options: &Options) -> Result<(CommandBuilder, Option<tempfile::TempDir>)> {
    anyhow::ensure!(
        options.stdin.is_none() || matches!(options.launch, Launch::ShellCommand { .. }),
        "Captured stdin requires a shell-command run"
    );
    let mut startup = None;
    let mut cmd = match &options.launch {
        Launch::Command(argv) => {
            if argv.is_empty() {
                bail!("A command is required");
            }
            let mut c = CommandBuilder::new(&argv[0]);
            c.args(&argv[1..]);
            c
        }
        Launch::ShellCommand { shell, command } => {
            let mut c = if let Some(input) = &options.stdin {
                let dir = tempfile::Builder::new().prefix("tide-input-").tempdir()?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
                let path = dir.path().join("stdin");
                std::fs::write(&path, input)?;
                let mut c = CommandBuilder::new("/bin/sh");
                // Positional arguments preserve shell code and paths without interpolating either.
                c.args(["-c", "exec \"$0\" -c \"$1\" < \"$2\"", shell, command]);
                c.arg(path);
                startup = Some(dir);
                c
            } else {
                CommandBuilder::new(shell)
            };
            if options.stdin.is_none() {
                c.args(["-c", command]);
            }
            c
        }
        Launch::Shell { path, clean } => {
            let dir = tempfile::Builder::new().prefix("tide-shell-").tempdir()?;
            let name = PathBuf::from(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut c = CommandBuilder::new(path);
            c.env("TIDE_SHELL", path);
            match name.as_str() {
                "bash" => {
                    let mut rc = String::new();
                    if !clean {
                        rc.push_str("[[ -f ~/.bashrc ]] && source ~/.bashrc\n");
                    }
                    if options.private {
                        rc.push_str("HISTFILE=/dev/null\n");
                    }
                    // ble.sh saves stderr while loading and restores it on attach.
                    // Install the stream after shell job-control initialization but
                    // before ble.sh takes its descriptor snapshot.
                    rc.push_str("if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then exec 2>\"$TIDE_STDERR_TTY\"; fi\n");
                    if options.suggestions {
                        c.env("TIDE_BLE", asset("blesh/ble.sh")?);
                        rc.push_str("if ! declare -F ble-attach >/dev/null; then source \"$TIDE_BLE\" --noattach; fi\n");
                    }
                    rc.push_str(include_str!("../shell/bash.sh"));
                    if options.suggestions {
                        rc.push_str("\nble-attach\n");
                    }
                    std::fs::write(dir.path().join("bashrc"), rc)?;
                    c.arg("--rcfile");
                    c.arg(dir.path().join("bashrc"));
                    c.arg("-i");
                }
                "zsh" => {
                    let original = std::env::var("ZDOTDIR")
                        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
                    c.env("TIDE_ORIGINAL_ZDOTDIR", &original);
                    if !clean {
                        std::fs::write(
                            dir.path().join(".zshenv"),
                            "[[ -f $TIDE_ORIGINAL_ZDOTDIR/.zshenv ]] && source $TIDE_ORIGINAL_ZDOTDIR/.zshenv\n",
                        )?;
                    }
                    let mut rc = String::new();
                    if !clean {
                        rc.push_str("[[ -f $TIDE_ORIGINAL_ZDOTDIR/.zshrc ]] && source $TIDE_ORIGINAL_ZDOTDIR/.zshrc\n");
                    }
                    if options.private {
                        rc.push_str("HISTFILE=/dev/null\nSAVEHIST=0\n");
                    }
                    if options.suggestions {
                        c.env(
                            "TIDE_ZSH_SUGGESTIONS",
                            asset("zsh-autosuggestions/zsh-autosuggestions.zsh")?,
                        );
                        rc.push_str("if (( ! $+functions[_zsh_autosuggest_start] )); then source \"$TIDE_ZSH_SUGGESTIONS\"; fi\n");
                    }
                    rc.push_str(include_str!("../shell/zsh.sh"));
                    std::fs::write(dir.path().join(".zshrc"), rc)?;
                    c.env("ZDOTDIR", dir.path());
                    c.arg("-i");
                }
                "fish" => {
                    let path = dir.path().join("integration.fish");
                    std::fs::write(&path, include_str!("../shell/fish.fish"))?;
                    c.env("TIDE_INTEGRATION", &path);
                    if *clean {
                        c.arg("--no-config");
                    }
                    if options.private {
                        c.arg("--private");
                    }
                    c.args([
                        "--interactive",
                        "--init-command",
                        "source \"$TIDE_INTEGRATION\"",
                    ]);
                }
                _ => {
                    c.arg("-i");
                }
            }
            if *clean {
                c.env("PS1", "tide> ");
                c.env("PROMPT", "tide> ");
            }
            c.env("TIDE_SELECTION_FILE", dir.path().join("selection"));
            startup = Some(dir);
            c
        }
    };
    cmd.cwd(&options.cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "tide-terminal");
    cmd.env("TIDE_EXE", std::env::current_exe()?);
    if let Some(path) = &options.history {
        cmd.env("TIDE_HISTORY", path);
    }
    if options.private {
        cmd.env("TIDE_PRIVATE", "1");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("fish_history", "");
    }
    for (k, v) in &options.env {
        cmd.env(k, v);
    }
    Ok((cmd, startup))
}

fn asset(name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let parent = exe.parent().unwrap();
    let roots = [
        std::env::var_os("TIDE_ASSETS_DIR").map(PathBuf::from),
        Some(parent.join("vendor")),
        parent.parent().map(|p| p.join("vendor")),
        parent
            .parent()
            .map(|p| p.join("share/tide-terminal/vendor")),
        if option_env!("TIDE_PACKAGED") == Some("1") {
            None
        } else {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor"))
        },
    ];
    for root in roots.into_iter().flatten() {
        let p = root.join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    bail!(
        "Bundled shell suggestions not found. Reinstall Tide, keep vendor/ beside the app, or disable enhanced suggestions in Settings."
    )
}

impl Session {
    pub fn spawn(options: Options) -> Result<Self> {
        let rules = Rules::new(options.rules.clone())?;
        let history = if options.private {
            None
        } else {
            options
                .history
                .as_ref()
                .map(|p| History::open(p))
                .transpose()?
        };
        let mut recorder = SplitCapture::new(
            options.managed().then(|| options.label()),
            options.cwd.clone(),
            match &options.launch {
                Launch::Shell { path, .. } => path.clone(),
                Launch::ShellCommand { shell, .. } => shell.clone(),
                Launch::Command(_) => std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()),
            },
            options.private,
        );
        let append_server = if options.managed() {
            None
        } else {
            Some(crate::append_cli::SnapshotServer::with_gate(
                recorder.completed.clone(),
                recorder.ready.clone(),
            )?)
        };
        let token = platform::integration_token()?;
        recorder.require_integration_token(Some(token.clone()));
        let (mut command, startup) = setup_command(&options)?;
        command.env("TIDE_MARKER_TOKEN", &token);
        if let Some(server) = &append_server {
            command.env("TIDE_APPEND_SOCKET", &server.path);
        }
        let selection = startup.as_ref().map(|d| d.path().join("selection"));
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: options.rows.max(2),
            cols: options.cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let stderr_pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: options.rows.max(2),
            cols: options.cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let hold = split_pty::redirect(
            &mut command,
            stderr_pair.master.as_ref(),
            matches!(&options.launch,Launch::Shell{path,..} if matches!(std::path::Path::new(path).file_name().and_then(|p|p.to_str()),Some("bash"|"zsh"))),
        )?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .context("Could not launch terminal process")?;
        drop(pair.slave);
        drop(stderr_pair.slave);
        let pid = child
            .process_id()
            .context("The PTY did not return a process ID")?;
        let processes = match crate::processes::SessionProcesses::new(pid) {
            Ok(processes) => Arc::new(Mutex::new(processes)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error)
                    .context("Could not track terminal processes (Linux pidfds required)");
            }
        };
        let worker_processes = processes.clone();
        let writer = AsyncWriter::new(pair.master.take_writer()?);
        let stderr_writer = AsyncWriter::new(stderr_pair.master.take_writer()?);
        let (read_tx, read_rx) = bounded(64);
        let reader = split_pty::Reader::start(
            pair.master.as_ref(),
            stderr_pair.master.as_ref(),
            hold,
            read_tx,
        )?;
        let (terminal, core_events) =
            terminal::create(options.rows.max(2) as usize, options.cols.max(2) as usize);
        let id = next_id();
        let run_id = next_id();
        let state = Arc::new(Mutex::new(State {
            recording: recorder.current.clone(),
            terminal,
            title: options.shell_name(),
            cwd: options.cwd.to_string_lossy().into_owned(),
            command: if options.managed() {
                options.label()
            } else {
                String::new()
            },
            last_command: if options.managed() {
                options.label()
            } else {
                String::new()
            },
            run_id,
            running: true,
            at_prompt: false,
            managed: options.managed(),
            attention: None,
            stop_reason: None,
            exit_code: None,
            exit_signal: None,
            elapsed: Duration::ZERO,
            changed: 0,
        }));
        let (tx, rx) = bounded(256);
        let (event_tx, events) = bounded(1024);
        let shared = state.clone();
        let wake: Waker = Arc::new(Mutex::new(None));
        let worker_wake = wake.clone();
        let worker = std::thread::Builder::new()
            .name(format!("tide-session-{id}"))
            .spawn(move || {
                let _startup = startup;
                let _append_server = append_server;
                let mut w = Worker {
                    options,
                    processes: worker_processes,
                    child,
                    _reader: reader,
                    stderr_master: stderr_pair.master,
                    master: pair.master,
                    writer,
                    stderr_writer,
                    read_rx,
                    rx,
                    tx: event_tx,
                    core_events,
                    state: shared,
                    pid,
                    run_id,
                    rules,
                    history,
                    observer: Observer::with_integration_token(Some(token)),
                    recorder,
                    capture_parser: std::array::from_fn(|_| vte::Parser::new()),
                    frames: std::array::from_fn(|_| crate::stream_frame::Frames::default()),
                    end_pending: None,
                    prompt_pending: false,
                    parser: vte::Parser::new(),
                    processor: ansi::Processor::new(),
                    started: platform::elapsed_clock(),
                    last_output: platform::elapsed_clock(),
                    pending_command: String::new(),
                    hint: None,
                    last_attention: String::new(),
                    stopping: None,
                    exit: None,
                    exit_signal: None,
                    eof: false,
                    closing: false,
                    wake: worker_wake,
                    reported_attention: false,
                    protocol_guard: std::array::from_fn(|_| ProtocolGuard::default()),
                };
                w.run();
            })?;
        Ok(Self {
            id,
            state,
            events,
            tx,
            worker: Some(worker),
            wake,
            selection,
            processes,
        })
    }
    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> Result<()> {
        self.tx
            .try_send(Control::Input(bytes.into()))
            .context("Terminal input queue is full or closed")
    }
    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.tx.try_send(Control::Resize(rows.max(2), cols.max(2)));
    }
    pub fn stop(&self) {
        let _ = self.tx.try_send(Control::Stop);
    }
    pub fn snapshot(&self) -> terminal::View {
        terminal::snapshot(&self.state.lock().unwrap().terminal)
    }
    pub fn scroll(&self, lines: i32) {
        terminal::scroll(&mut self.state.lock().unwrap().terminal, lines);
    }
    pub fn select(&self, a: (usize, usize), b: (usize, usize)) -> String {
        terminal::selected(&self.state.lock().unwrap().terminal, a, b)
    }
    pub fn is_running(&self) -> bool {
        self.state.lock().unwrap().running
    }
    /// Processes still owned by this terminal, including background jobs.
    pub fn active_processes(&self) -> Vec<String> {
        let (managed, running, at_prompt, command) = {
            let state = self.state.lock().unwrap();
            (
                state.managed,
                state.running,
                state.at_prompt,
                state.command.clone(),
            )
        };
        let mut names = self.processes.lock().unwrap().active_names(managed);
        if names.is_empty() && running && !at_prompt {
            names.push(if command.is_empty() {
                "Shell is still starting or running".into()
            } else {
                command
            });
        }
        names
    }
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.lock().unwrap() = Some(Box::new(wake));
    }
    pub fn insert_command(&self, command: &str) -> Result<()> {
        anyhow::ensure!(
            self.state.lock().unwrap().at_prompt,
            "The terminal must be at its shell prompt"
        );
        let path = self
            .selection
            .as_ref()
            .context("This session has no shell editing integration")?;
        std::fs::write(path, command)?;
        self.write(b"\x18\x19".to_vec())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.processes.lock().unwrap().shutdown();
        let _ = self.tx.send(Control::Close);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker {
    processes: Arc<Mutex<crate::processes::SessionProcesses>>,
    _reader: split_pty::Reader,
    stderr_master: Box<dyn MasterPty + Send>,
    recorder: SplitCapture,
    capture_parser: [vte::Parser; 2],
    frames: [crate::stream_frame::Frames; 2],
    end_pending: Option<i32>,
    prompt_pending: bool,
    options: Options,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: AsyncWriter,
    stderr_writer: AsyncWriter,
    read_rx: Receiver<ReadEvent>,
    rx: Receiver<Control>,
    tx: Sender<Event>,
    core_events: Receiver<CoreEvent>,
    state: Arc<Mutex<State>>,
    pid: u32,
    run_id: u64,
    rules: Rules,
    history: Option<History>,
    observer: Observer,
    parser: vte::Parser,
    processor: ansi::Processor,
    started: Duration,
    last_output: Duration,
    pending_command: String,
    hint: Option<(String, Instant)>,
    last_attention: String,
    stopping: Option<(Duration, bool)>,
    exit: Option<(u32, Instant)>,
    exit_signal: Option<String>,
    eof: bool,
    closing: bool,
    wake: Waker,
    reported_attention: bool,
    protocol_guard: [ProtocolGuard; 2],
}
impl Worker {
    fn emit(&self, event: Event) {
        // Output already reached the terminal model. Notification consumers must never freeze supervision.
        if self.tx.try_send(event).is_err() { /* UI may have closed; supervision continues independently. */
        }
        if let Some(wake) = &*self.wake.lock().unwrap() {
            wake();
        }
    }
    fn record(&self, command: &str, code: i32) {
        if let Some(h) = &self.history {
            let cwd = self.state.lock().unwrap().cwd.clone();
            if let Err(e) = h.record(command, &cwd, &self.options.shell_name(), code) {
                self.emit(Event::Error {
                    message: format!("History: {e}"),
                });
            }
        }
    }
    fn attention(&mut self, message: String, reported: bool) {
        if message == self.last_attention {
            return;
        }
        self.last_attention = message.clone();
        self.reported_attention = reported;
        self.state.lock().unwrap().attention = Some(message.clone());
        self.emit(Event::Attention {
            run_id: self.run_id,
            message: message.clone(),
            reported,
        });
        if self.options.notifications {
            platform::notify(
                "Tide · input needed",
                &format!("{} — {}", self.options.shell_name(), message),
            );
        }
    }
    fn clear_attention(&mut self) {
        self.hint = None;
        self.last_attention.clear();
        self.reported_attention = false;
        self.state.lock().unwrap().attention = None;
        self.emit(Event::AttentionCleared);
    }
    fn request_stop(&mut self, reason: String, closing: bool) {
        if self.exit.is_some() {
            // A descendant may still produce output after the direct child exits.
            // Explicit close must remain able to end draining and drop the PTYs.
            self.closing |= closing;
            return;
        }
        if self.stopping.is_some() {
            if closing {
                self.closing = true;
                self.stopping = Some((platform::elapsed_clock(), false));
            }
            return;
        }
        if !self.options.managed() && !closing {
            self.emit(Event::Error {
                message: "Automatic stopping requires a managed run. Use Run with rules.".into(),
            });
            return;
        }
        if closing {
            // Explicitly closing a terminal also closes its current foreground job.
            if let Some(group) = self.master.process_group_leader()
                && group > 1
                && group != self.pid as i32
            {
                let _ = platform::signal_group(group as u32, libc::SIGHUP);
            }
        }
        match platform::signal_group(self.pid, if closing { libc::SIGHUP } else { libc::SIGINT }) {
            Ok(()) => {}
            Err(e) => self.emit(Event::Error {
                message: format!("Could not signal owned job: {e}"),
            }),
        }
        self.state.lock().unwrap().stop_reason = Some(reason.clone());
        self.emit(Event::Stopping {
            run_id: self.run_id,
            reason,
        });
        self.closing = closing;
        self.stopping = Some((platform::elapsed_clock(), false));
    }
    /// Some pagers read ttyname(2), not stdin or /dev/tty. When only
    /// that terminal enters noncanonical/no-echo mode it owns interactive input.
    /// Send each byte to one terminal only, never duplicate input into the shell.
    fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let primary = self.master.get_termios();
        let secondary = self.stderr_master.get_termios();
        let use_secondary = match (&primary, &secondary) {
            (Some(a), Some(b)) => {
                let a = a.local_flags.bits();
                let b = b.local_flags.bits();
                (a & libc::ICANON != 0 && b & libc::ICANON == 0)
                    || (a & libc::ECHO != 0 && b & libc::ECHO == 0)
            }
            _ => false,
        };
        if !use_secondary {
            return self.writer.write_all(bytes);
        }
        let (primary, secondary) = (primary.unwrap(), secondary.unwrap());
        if primary.local_flags.bits() & libc::ISIG != 0
            && secondary.local_flags.bits() & libc::ISIG != 0
        {
            // Only the controlling terminal can signal the foreground group.
            // Preserve configured interrupt/quit/suspend characters for less.
            let mut start = 0;
            for (index, byte) in bytes.iter().enumerate() {
                if let Some(control) =
                    [libc::VINTR, libc::VQUIT, libc::VSUSP]
                        .into_iter()
                        .find(|control| {
                            secondary.control_chars[*control] != 0
                                && secondary.control_chars[*control] == *byte
                        })
                {
                    if start < index {
                        self.stderr_writer.write_all(&bytes[start..index])?;
                    }
                    self.writer.write_all(&[primary.control_chars[control]])?;
                    start = index + 1;
                }
            }
            if start < bytes.len() {
                self.stderr_writer.write_all(&bytes[start..])?;
            }
            Ok(())
        } else {
            self.stderr_writer.write_all(bytes)
        }
    }
    fn output(&mut self, channel: Channel, bytes: &[u8]) {
        let filtered = self.protocol_guard[channel as usize].filter(bytes);
        let bytes = filtered.as_slice();
        self.recorder.select(channel);
        self.capture_parser[channel as usize].advance(&mut self.recorder, bytes);
        self.state.lock().unwrap().recording = self.recorder.current.clone();
        let framed = self.frames[channel as usize].filter(bytes);
        let bytes = framed.as_slice();
        if !bytes.is_empty() {
            self.emit(Event::Output {
                text: String::from_utf8_lossy(bytes).into_owned(),
            });
        }
        {
            let mut s = self.state.lock().unwrap();
            self.processor.advance(&mut s.terminal, bytes);
            s.changed += 1;
        }
        self.observer.visible_activity = false;
        self.parser.advance(&mut self.observer, bytes);
        if self.observer.visible_activity {
            self.last_output = platform::elapsed_clock();
        }
        while let Ok(event) = self.core_events.try_recv() {
            match event {
                CoreEvent::PtyWrite(s) => {
                    let _ = self.write_input(s.as_bytes());
                }
                CoreEvent::Title(title) => self.state.lock().unwrap().title = title,
                CoreEvent::ColorRequest(index, f) => {
                    let configured = self.state.lock().unwrap().terminal.colors()[index];
                    let [r, g, b] = match configured {
                        Some(rgb) => [rgb.r, rgb.g, rgb.b],
                        None => terminal::color(match index {
                            0..=255 => ansi::Color::Indexed(index as u8),
                            257 => ansi::Color::Named(ansi::NamedColor::Background),
                            258 => ansi::Color::Named(ansi::NamedColor::Cursor),
                            _ => ansi::Color::Named(ansi::NamedColor::Foreground),
                        }),
                    };
                    let _ = self.write_input(f(ansi::Rgb { r, g, b }).as_bytes());
                }
                CoreEvent::TextAreaSizeRequest(f) => {
                    let s = self.state.lock().unwrap();
                    let v = terminal::snapshot(&s.terminal);
                    let response = f(alacritty_terminal::event::WindowSize {
                        num_lines: v.rows as u16,
                        num_cols: v.cols as u16,
                        cell_width: 9,
                        cell_height: 20,
                    });
                    drop(s);
                    let _ = self.write_input(response.as_bytes());
                }
                _ => {}
            }
        }
        let mut markers = std::mem::take(&mut self.observer.markers);
        // Only stdout carries authoritative shell events; stderr has mirrored
        // boundaries for capture and shell-native decorations, not new jobs.
        if channel == Channel::Stderr {
            markers.retain(|marker| matches!(marker, Marker::Input(_)));
        }
        let capture_done = self.recorder.ready.load(Ordering::Acquire);
        if capture_done {
            if let Some(code) = self.end_pending.take() {
                markers.insert(0, Marker::End((code >= 0).then_some(code)));
            }
            if self.prompt_pending {
                markers.push(Marker::Prompt);
                self.prompt_pending = false;
            }
        }
        for marker in markers {
            match marker {
                Marker::Cwd(cwd) => self.state.lock().unwrap().cwd = cwd,
                Marker::Command(cmd) => {
                    let mut s = self.state.lock().unwrap();
                    if !self.options.managed() && !s.at_prompt {
                        // Fish may emit its native OSC 133 C before fish_preexec invokes our hook.
                        if s.command.is_empty() {
                            s.command = cmd.clone();
                            if !cmd.trim().is_empty() {
                                s.last_command = cmd.clone();
                            }
                            drop(s);
                            self.emit(Event::CommandStarted {
                                run_id: self.run_id,
                                command: cmd,
                            });
                        }
                    } else {
                        self.pending_command = cmd;
                    }
                }
                Marker::Start if !self.options.managed() => {
                    // Duplicate standard markers from shell plugins must not manufacture new runs.
                    if !self.state.lock().unwrap().at_prompt {
                        continue;
                    }
                    self.run_id = next_id();
                    self.started = platform::elapsed_clock();
                    self.observer.clear_text();
                    self.clear_attention();
                    let cmd = std::mem::take(&mut self.pending_command);
                    {
                        let mut s = self.state.lock().unwrap();
                        s.command = cmd.clone();
                        if !cmd.trim().is_empty() {
                            s.last_command = cmd.clone();
                        }
                        s.at_prompt = false;
                        s.run_id = self.run_id;
                    }
                    self.emit(Event::CommandStarted {
                        run_id: self.run_id,
                        command: cmd,
                    });
                }
                Marker::End(code) if !self.options.managed() => {
                    let code = code.unwrap_or(-1);
                    if !capture_done {
                        self.end_pending = Some(code);
                        continue;
                    }
                    // Appending deliberately preserves its source capture. Only use the
                    // coordinated status when it belongs to this completed command.
                    let code = self
                        .recorder
                        .completed
                        .lock()
                        .unwrap()
                        .as_ref()
                        .filter(|run| run.command == self.state.lock().unwrap().command)
                        .and_then(|run| run.exit_code)
                        .unwrap_or(code);
                    let cmd = {
                        let mut s = self.state.lock().unwrap();
                        s.at_prompt = true;
                        std::mem::take(&mut s.command)
                    };
                    if !cmd.is_empty() {
                        self.record(&cmd, code);
                        self.emit(Event::CommandEnded {
                            run_id: self.run_id,
                            code,
                        });
                    }
                    self.clear_attention();
                }
                Marker::Prompt if !self.options.managed() => {
                    if !capture_done {
                        self.prompt_pending = true;
                        continue;
                    }
                    self.state.lock().unwrap().at_prompt = true;
                    self.clear_attention();
                    self.emit(Event::Prompt);
                }
                Marker::Input(_) => self.attention("Input requested".into(), true),
                _ => {}
            }
        }
        let at_prompt = self.state.lock().unwrap().at_prompt;
        if !at_prompt && !self.reported_attention {
            if let Some(hint) = self.observer.input_hint() {
                if self.hint.as_ref().is_none_or(|(s, _)| s != hint) {
                    self.hint = Some((hint.into(), Instant::now()));
                }
            } else if self.hint.is_some() || !self.last_attention.is_empty() {
                self.clear_attention();
            }
        }
    }
    fn run(&mut self) {
        self.emit(Event::Started {
            run_id: self.run_id,
            pid: self.pid,
            command: self.options.label(),
            managed: self.options.managed(),
        });
        let tick = crossbeam_channel::tick(Duration::from_millis(25));
        loop {
            select! {
                recv(self.rx)->control=>match control{
                    Ok(Control::Input(bytes))=>{if self.exit.is_none(){self.clear_attention();self.state.lock().unwrap().terminal.scroll_display(alacritty_terminal::grid::Scroll::Bottom);if let Err(e)=self.write_input(&bytes){self.emit(Event::Error{message:e.to_string()});}}},
                    Ok(Control::Resize(rows,cols))=>{
                        let _=self.master.resize(PtySize{rows,cols,pixel_width:0,pixel_height:0});let _=self.stderr_master.resize(PtySize{rows,cols,pixel_width:0,pixel_height:0});let mut s=self.state.lock().unwrap();s.terminal.resize(terminal::Size{rows:rows as usize,cols:cols as usize});s.changed+=1;
                    },
                    Ok(Control::Stop)=>self.request_stop("Stopped by user".into(),false),
                    Ok(Control::Close)|Err(_)=>self.request_stop("Terminal closed".into(),true),
                },
                recv(self.read_rx)->read=>match read{Ok(ReadEvent::Bytes(channel,bytes))=>self.output(channel,&bytes),_=>{self.eof=true;self.read_rx=crossbeam_channel::never();}},
                recv(tick)->_=>{}
            }
            let now = platform::elapsed_clock();
            let child_exited =
                self.exit.is_none() && crate::processes::child_exited(self.pid).unwrap_or(true);
            {
                let mut processes = self.processes.lock().unwrap();
                if child_exited || (self.exit.is_some() && processes.refresh_due()) {
                    processes.refresh();
                }
            }
            if child_exited {
                match self.child.try_wait() {
                    Ok(Some(status)) => {
                        self.exit_signal = status.signal().map(str::to_owned);
                        self.exit = Some((status.exit_code(), Instant::now()));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.emit(Event::Error {
                            message: e.to_string(),
                        });
                        self.exit = Some((1, Instant::now()));
                    }
                }
            }
            if self.exit.is_none() {
                for error in self
                    .writer
                    .errors
                    .try_iter()
                    .chain(self.stderr_writer.errors.try_iter())
                {
                    if self.stopping.is_none() {
                        self.emit(Event::Error { message: error });
                    }
                }
                self.state.lock().unwrap().elapsed = now.saturating_sub(self.started);
                if self.options.managed()
                    && self.stopping.is_none()
                    && let Some(reason) = self.rules.evaluate(
                        now.saturating_sub(self.started),
                        now.saturating_sub(self.last_output),
                        &self.observer.text(),
                    )
                {
                    self.emit(Event::RuleTriggered {
                        run_id: self.run_id,
                        reason: reason.clone(),
                    });
                    self.request_stop(reason, false);
                }
                if let Some((since, escalated)) = self.stopping {
                    let grace = if self.closing {
                        Duration::from_millis(250)
                    } else {
                        self.rules.grace()
                    };
                    if !escalated
                        && (self.rules.force() || self.closing)
                        && now.saturating_sub(since) >= grace
                    {
                        if let Err(e) = platform::signal_group(self.pid, libc::SIGKILL) {
                            self.emit(Event::Error {
                                message: e.to_string(),
                            });
                        }
                        self.emit(Event::Escalated {
                            run_id: self.run_id,
                        });
                        self.stopping = Some((since, true));
                    }
                }
                if let Some((message, since)) = &self.hint
                    && since.elapsed() >= Duration::from_millis(350)
                    && self.stopping.is_none()
                {
                    self.attention(message.clone(), false);
                }
            }
            if let Some((code, when)) = self.exit
                && output_drained(
                    self.eof,
                    when.elapsed(),
                    self.read_rx.is_empty(),
                    self.closing,
                )
            {
                self.recorder.finish(code as i32);
                if self.options.managed() {
                    self.record(&self.options.label(), code as i32);
                }
                let reason = {
                    let mut s = self.state.lock().unwrap();
                    s.running = false;
                    s.exit_code = Some(code);
                    s.exit_signal = self.exit_signal.clone();
                    s.attention = None;
                    s.elapsed = now.saturating_sub(self.started);
                    s.stop_reason.clone()
                };
                self.emit(Event::Exited {
                    run_id: self.run_id,
                    code,
                    signal: self.exit_signal.clone(),
                    reason,
                });
                break;
            }
        }
    }
}

// Quiet descendants may retain a PTY indefinitely. Keep the existing grace for that
// case, but never discard queued output merely because parsing took longer than it.
fn output_drained(eof: bool, exit_age: Duration, queue_empty: bool, closing: bool) -> bool {
    eof || (exit_age > Duration::from_millis(300) && (queue_empty || closing))
}

/// A child that is not reading stdin must not block deadline enforcement.
struct AsyncWriter {
    tx: Sender<Vec<u8>>,
    errors: Receiver<String>,
}
impl AsyncWriter {
    fn new(mut writer: Box<dyn Write + Send>) -> Self {
        let (tx, rx) = bounded::<Vec<u8>>(16);
        let (error_tx, errors) = bounded(4);
        std::thread::spawn(move || {
            for bytes in rx {
                if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    let _ = error_tx.try_send(error.to_string());
                    break;
                }
            }
        });
        Self { tx, errors }
    }
}
impl Write for AsyncWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > 1_048_576 {
            return Err(std::io::Error::other("Paste exceeds the 1 MiB input limit"));
        }
        self.tx.try_send(bytes.to_vec()).map_err(|_| {
            std::io::Error::other(
                "The program is not consuming input; input queue is full or closed",
            )
        })?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod drain_tests {
    use super::output_drained;
    use std::time::Duration;

    #[test]
    fn queued_output_survives_exit_grace_until_drained_or_explicitly_closed() {
        let grace = Duration::from_millis(300);
        assert!(!output_drained(false, grace, true, false));
        assert!(!output_drained(
            false,
            grace + Duration::from_nanos(1),
            false,
            false
        ));
        assert!(!output_drained(
            false,
            Duration::from_secs(10),
            false,
            false
        ));
        assert!(output_drained(
            false,
            grace + Duration::from_nanos(1),
            true,
            false
        ));
        assert!(output_drained(false, Duration::from_secs(1), false, true));
        assert!(output_drained(true, Duration::ZERO, false, false));
    }
}
