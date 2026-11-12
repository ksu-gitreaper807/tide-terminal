//! Independent parser and command boundaries for each physical terminal channel.
use crate::{
    capture::{CAPTURE_LIMIT, CapturedRun, Recording, StreamOutput},
    observer::{Marker, Observer},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use vte::{Params, Perform};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Channel {
    Stdout = 0,
    Stderr = 1,
}
struct Input {
    sequence: u64,
    active: Option<u64>,
    pending: String,
    cwd: PathBuf,
}
struct Pending {
    record: Recording,
    combined: Observer,
    text: [Observer; 2],
    done: [bool; 2],
    statuses: [Option<i32>; 2],
    cancelled: bool,
}
pub struct SplitCapture {
    pub current: Option<Recording>,
    pub completed: Arc<Mutex<Option<CapturedRun>>>,
    pub ready: Arc<AtomicBool>,
    last: Option<Recording>,
    published: u64,
    discard_before: u64,
    runs: BTreeMap<u64, Pending>,
    inputs: [Input; 2],
    selected: usize,
    shell: String,
    private: bool,
    managed: bool,
    metadata: Observer,
}
impl SplitCapture {
    pub fn new(command: Option<String>, cwd: PathBuf, shell: String, private: bool) -> Self {
        let managed = command.is_some();
        let mut this = Self {
            current: None,
            completed: Arc::new(Mutex::new(None)),
            ready: Arc::new(AtomicBool::new(!managed)),
            last: None,
            published: 0,
            discard_before: 0,
            runs: BTreeMap::new(),
            inputs: std::array::from_fn(|_| Input {
                sequence: 0,
                active: None,
                pending: String::new(),
                cwd: cwd.clone(),
            }),
            selected: 0,
            shell,
            private,
            managed,
            metadata: Observer::default(),
        };
        if let Some(command) = command {
            for channel in 0..2 {
                this.selected = channel;
                this.inputs[channel].pending = command.clone();
                this.start();
            }
        }
        this
    }
    pub fn require_integration_token(&mut self, token: Option<String>) {
        self.metadata = Observer::with_integration_token(token);
    }
    pub fn select(&mut self, channel: Channel) {
        self.selected = channel as usize;
    }
    fn start(&mut self) {
        let input = &mut self.inputs[self.selected];
        if input.active.is_some() {
            return;
        }
        input.sequence += 1;
        let id = input.sequence;
        input.active = Some(id);
        let command = std::mem::take(&mut input.pending);
        if id < self.discard_before {
            return;
        }
        // A stalled/malformed channel must not accumulate unbounded command buffers.
        // Mark evicted recordings unavailable so pinned GUI actions fail explicitly.
        if !self.runs.contains_key(&id)
            && self.runs.len() >= 16
            && let Some((oldest, run)) = self.runs.pop_first()
        {
            run.record.lock().unwrap().exit_code = Some(-1);
            self.discard_before = oldest + 1;
        }
        let run = self.runs.entry(id).or_insert_with(|| Pending {
            record: Arc::new(Mutex::new(CapturedRun {
                command: command.clone(),
                cwd: input.cwd.clone(),
                shell: self.shell.clone(),
                private: self.private,
                output: String::new(),
                streams: None,
                truncated: false,
                exit_code: None,
            })),
            combined: Observer::with_text_limit(CAPTURE_LIMIT),
            text: std::array::from_fn(|_| Observer::with_text_limit(CAPTURE_LIMIT)),
            done: [false; 2],
            statuses: [None; 2],
            cancelled: false,
        });
        if !command.is_empty() && run.record.lock().unwrap().command.is_empty() {
            run.record.lock().unwrap().command = command;
        }
        if id >= self.inputs[1 - self.selected].sequence {
            self.current = Some(run.record.clone());
        }
        self.ready.store(false, Ordering::Release);
    }
    fn end(&mut self, code: Option<i32>, cancel: bool) {
        let Some(id) = self.inputs[self.selected].active.take() else {
            return;
        };
        let Some(run) = self.runs.get_mut(&id) else {
            return;
        };
        run.done[self.selected] = true;
        run.statuses[self.selected] = code;
        run.cancelled |= cancel;
        if run.done.iter().all(|done| *done) {
            let run = self.runs.remove(&id).unwrap();
            if !run.cancelled {
                let (combined, truncated) = bounded_text(&run.combined);
                let (stdout, stdout_truncated) = bounded_text(&run.text[0]);
                let (stderr, stderr_truncated) = bounded_text(&run.text[1]);
                let mut record = run.record.lock().unwrap();
                record.output = combined;
                record.truncated = truncated;
                record.streams = Some(StreamOutput {
                    stdout,
                    stderr,
                    stdout_truncated,
                    stderr_truncated,
                });
                record.exit_code = Some(match run.statuses {
                    [Some(a), Some(b)] if a == b => a,
                    _ => -1, // Completed, but status is unknown or channels disagree.
                });
                if id >= self.published {
                    self.published = id;
                    *self.completed.lock().unwrap() = Some(record.clone());
                    self.last = Some(run.record.clone());
                }
            } else if self.runs.is_empty() {
                self.current = self.last.clone();
            }
        }
        self.ready.store(self.runs.is_empty(), Ordering::Release);
    }
    pub fn finish(&mut self, code: i32) {
        for channel in 0..2 {
            self.selected = channel;
            self.end(self.managed.then_some(code), false);
        }
    }
    fn text(&mut self, f: impl Fn(&mut Observer)) {
        if let Some(id) = self.inputs[self.selected].active
            && let Some(run) = self.runs.get_mut(&id)
        {
            f(&mut run.combined);
            f(&mut run.text[self.selected]);
        }
    }
}
fn bounded_text(observer: &Observer) -> (String, bool) {
    let mut text = observer.text();
    let mut trim = text.len().saturating_sub(CAPTURE_LIMIT);
    while !text.is_char_boundary(trim) {
        trim += 1;
    }
    text.drain(..trim);
    (text, observer.truncated() || trim > 0)
}
impl Perform for SplitCapture {
    fn print(&mut self, c: char) {
        self.text(|text| text.print(c));
    }
    fn execute(&mut self, b: u8) {
        self.text(|text| text.execute(b));
    }
    fn csi_dispatch(&mut self, p: &Params, i: &[u8], ignore: bool, a: char) {
        self.text(|text| text.csi_dispatch(p, i, ignore, a));
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], bell: bool) {
        self.metadata.osc_dispatch(params, bell);
        for marker in std::mem::take(&mut self.metadata.markers) {
            match marker {
                Marker::Append if !self.managed => self.end(None, true),
                Marker::Start if !self.managed => self.start(),
                Marker::End(code) if !self.managed => self.end(code, false),
                Marker::Command(command) if !self.managed => {
                    if let Some(id) = self.inputs[self.selected].active
                        && let Some(run) = self.runs.get(&id)
                    {
                        let mut record = run.record.lock().unwrap();
                        if record.command.is_empty() {
                            record.command = command;
                        }
                    } else {
                        self.inputs[self.selected].pending = command;
                    }
                }
                Marker::Cwd(cwd) => {
                    self.inputs[self.selected].cwd = PathBuf::from(&cwd);
                    if self.selected == 0
                        && self.inputs[self.selected].active.is_none()
                        && let Some(record) = &self.last
                    {
                        let mut record = record.lock().unwrap();
                        record.cwd = PathBuf::from(cwd);
                        *self.completed.lock().unwrap() = Some(record.clone());
                    }
                }
                _ => {}
            }
        }
    }
}
