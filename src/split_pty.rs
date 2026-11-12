//! A second output-only PTY retains isatty(2) without replacing stdout with a pipe.
use crate::split_capture::Channel;
use anyhow::{Context, Result};
use crossbeam_channel::{SendTimeoutError, Sender};
use portable_pty::{CommandBuilder, MasterPty};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub enum ReadEvent {
    Bytes(Channel, Vec<u8>),
    Eof,
}
pub fn redirect(
    command: &mut CommandBuilder,
    stderr: &dyn MasterPty,
    in_shell: bool,
) -> Result<File> {
    let path = stderr.tty_name().context("stderr PTY has no slave path")?;
    let hold = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(&path)?;
    command.env("TIDE_SPLIT_CAPTURE", "1");
    command.env("TIDE_STDERR_TTY", &path);
    if in_shell {
        return Ok(hold);
    }
    let argv = command.get_argv().clone();
    *command.get_argv_mut() = vec![
        "/bin/sh".into(),
        "-c".into(),
        "stderr=$1; shift; exec \"$@\" 2> \"$stderr\"".into(),
        "tide-stderr".into(),
        path.into_os_string(),
    ];
    command.get_argv_mut().extend(argv);
    command.env("TIDE_SPLIT_CAPTURE", "1");
    Ok(hold)
}
fn clone_fd(master: &dyn MasterPty) -> Result<File> {
    let fd = master.as_raw_fd().context("PTY has no native descriptor")?;
    // SAFETY: fcntl duplicates a live descriptor into independently owned storage.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: this new descriptor is owned by this function exactly once.
    Ok(unsafe { File::from_raw_fd(duplicate) })
}
pub struct Reader {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Reader {
    pub fn start(
        stdout: &dyn MasterPty,
        stderr: &dyn MasterPty,
        hold: File,
        tx: Sender<ReadEvent>,
    ) -> Result<Self> {
        let mut files = [clone_fd(stdout)?, clone_fd(stderr)?];
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("tide-stream-reader".into())
            .spawn(move || {
                let mut hold = Some(hold);
                let mut open = [true; 2];
                let mut buffer = [0; 8192];
                let send = |mut event| -> bool {
                    loop {
                        match tx.send_timeout(event, Duration::from_millis(20)) {
                            Ok(()) => return true,
                            Err(SendTimeoutError::Timeout(value))
                                if !stopping.load(Ordering::Acquire) =>
                            {
                                event = value
                            }
                            _ => return false,
                        }
                    }
                };
                while open.iter().any(|open| *open) && !stopping.load(Ordering::Acquire) {
                    let mut fds = std::array::from_fn::<_, 2, _>(|i| libc::pollfd {
                        fd: if open[i] { files[i].as_raw_fd() } else { -1 },
                        events: libc::POLLIN,
                        revents: 0,
                    });
                    // SAFETY: poll receives a valid two-element descriptor array.
                    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, 20) };
                    if ready < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break;
                    }
                    for i in 0..2 {
                        if fds[i].revents == 0 {
                            continue;
                        }
                        match files[i].read(&mut buffer) {
                            Ok(n) if n > 0 => {
                                if i == 1 {
                                    hold.take();
                                }
                                if !send(ReadEvent::Bytes(
                                    if i == 0 {
                                        Channel::Stdout
                                    } else {
                                        Channel::Stderr
                                    },
                                    buffer[..n].to_vec(),
                                )) {
                                    return;
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            _ => {
                                open[i] = false;
                                if i == 0 {
                                    hold.take();
                                }
                            }
                        }
                    }
                }
                let _ = send(ReadEvent::Eof);
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
