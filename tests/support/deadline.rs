//! Fail a stalled test even if it blocks inside spawn or a destructor.
use std::{
    io::Write,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct Deadline {
    tx: Sender<Option<&'static str>>,
    thread: Option<JoinHandle<()>>,
}
impl Deadline {
    pub fn new(name: &'static str, timeout: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("test-deadline".into())
            .spawn(move || {
                let deadline = Instant::now() + timeout;
                let mut stage = "starting";
                loop {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(Some(next)) => stage = next,
                        Ok(None) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
                // Direct writes bypass libtest's buffered output capture. Avoid
                // command lines, terminal output and input (including passwords).
                let mut log = std::io::stderr().lock();
                let _ = writeln!(
                    log,
                    "TEST DEADLINE: {name}, stage={stage}, limit={timeout:?}"
                );
                if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
                    for task in tasks.flatten() {
                        let _ = writeln!(log, "thread {}", task.file_name().to_string_lossy());
                        for field in ["comm", "wchan", "syscall", "children"] {
                            let value = std::fs::read_to_string(task.path().join(field))
                                .unwrap_or_else(|error| format!("unavailable: {error}"));
                            let _ = writeln!(log, "  {field}: {}", value.trim());
                        }
                    }
                }
                let _ = log.flush();
                // A panic in this thread would leave the blocked test running.
                // Exit nonzero; CI retains this diagnostic instead of hanging.
                std::process::exit(124);
            })
            .unwrap();
        Self {
            tx,
            thread: Some(thread),
        }
    }
    pub fn stage(&self, stage: &'static str) {
        self.tx.send(Some(stage)).unwrap();
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        let _ = self.tx.send(None);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
