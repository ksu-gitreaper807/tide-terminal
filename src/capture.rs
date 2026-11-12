//! Per-command, in-memory terminal-text capture. Output is published only at completion.
use crate::observer::{Marker, Observer};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use vte::{Params, Perform};

pub const CAPTURE_LIMIT: usize = 1_048_576;
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct StreamOutput {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}
pub type Recording = Arc<Mutex<CapturedRun>>;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CapturedRun {
    pub command: String,
    pub cwd: PathBuf,
    pub shell: String,
    pub private: bool,
    pub output: String,
    #[serde(default)]
    pub streams: Option<StreamOutput>,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}
pub struct Recorder {
    pub current: Option<Recording>,
    pub completed: Arc<Mutex<Option<CapturedRun>>>,
    last_completed: Option<Recording>,
    text: Observer,
    capturing: bool,
    managed: bool,
    pending: String,
    cwd: PathBuf,
    shell: String,
    private: bool,
}
impl Recorder {
    pub fn new(command: Option<String>, cwd: PathBuf, shell: String, private: bool) -> Self {
        let managed = command.is_some();
        let mut recorder = Self {
            current: None,
            completed: Arc::new(Mutex::new(None)),
            last_completed: None,
            text: Observer::with_text_limit(CAPTURE_LIMIT),
            capturing: false,
            managed,
            pending: String::new(),
            cwd,
            shell,
            private,
        };
        if let Some(command) = command {
            recorder.start(command);
        }
        recorder
    }
    fn start(&mut self, command: String) {
        self.text.clear_text();
        self.capturing = true;
        self.current = Some(Arc::new(Mutex::new(CapturedRun {
            command,
            cwd: self.cwd.clone(),
            shell: self.shell.clone(),
            private: self.private,
            output: String::new(),
            streams: None,
            truncated: false,
            exit_code: None,
        })));
    }
    pub fn finish(&mut self, code: i32) {
        if self.capturing {
            if let Some(current) = &self.current {
                let mut record = current.lock().unwrap();
                let mut output = self.text.text();
                let mut trim = output.len().saturating_sub(CAPTURE_LIMIT);
                while !output.is_char_boundary(trim) {
                    trim += 1;
                }
                record.truncated = self.text.truncated() || trim > 0;
                output.drain(..trim);
                record.output = output;
                record.exit_code = Some(code);
                *self.completed.lock().unwrap() = Some(record.clone());
                self.last_completed = Some(current.clone());
            }
            self.capturing = false;
        }
    }
}
impl Perform for Recorder {
    fn print(&mut self, c: char) {
        if self.capturing {
            self.text.print(c);
        }
    }
    fn execute(&mut self, b: u8) {
        if self.capturing {
            self.text.execute(b);
        }
    }
    fn csi_dispatch(&mut self, p: &Params, i: &[u8], ignore: bool, a: char) {
        if self.capturing {
            self.text.csi_dispatch(p, i, ignore, a);
        }
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], bell: bool) {
        // The shell function emits this before invoking the CLI. Its output/help/errors
        // must not replace the source, allowing several A_: operations on one recording.
        if !self.managed && params == [b"777".as_slice(), b"tide", b"append"] {
            self.capturing = false;
            self.pending.clear();
            self.current = self.last_completed.clone();
            return;
        }
        self.text.osc_dispatch(params, bell);
        for marker in std::mem::take(&mut self.text.markers) {
            match marker {
                Marker::Command(command) if !self.managed => {
                    if self.capturing {
                        if let Some(current) = &self.current {
                            let mut run = current.lock().unwrap();
                            if run.command.is_empty() {
                                run.command = command;
                            }
                        }
                    } else {
                        self.pending = command;
                    }
                }
                Marker::Start if !self.managed && !self.capturing => {
                    let command = std::mem::take(&mut self.pending);
                    self.start(command);
                }
                Marker::End(code) if !self.managed => self.finish(code),
                Marker::Cwd(cwd) => {
                    self.cwd = PathBuf::from(cwd);
                    if !self.capturing
                        && let Some(current) = &self.current
                    {
                        let mut record = current.lock().unwrap();
                        record.cwd = self.cwd.clone();
                        if record.exit_code.is_some() {
                            *self.completed.lock().unwrap() = Some(record.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
