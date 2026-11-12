use std::time::Duration;

/// Linux CLOCK_BOOTTIME is monotonic and includes system suspend.
pub fn elapsed_clock() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts points to initialized, writable storage; clock_gettime does not retain it.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    assert_eq!(rc, 0, "CLOCK_BOOTTIME unavailable");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub fn signal_group(pid: u32, signal: i32) -> std::io::Result<()> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(std::io::Error::other("Invalid owned process group"));
    }
    // SAFETY: the caller retains an unreaped child that owns this group, preventing PID reuse.
    // The negative argument targets only that child's process group, never the application group.
    let rc = unsafe { libc::kill(-(pid as i32), signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn notify(title: &str, message: &str) {
    let title = title.to_owned();
    let message = message.to_owned();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=Tide", "--", &title, &message])
            .status();
    });
}

pub struct RawTerminal(Option<libc::termios>);
impl RawTerminal {
    pub fn enter(enabled: bool) -> std::io::Result<Self> {
        if !enabled {
            return Ok(Self(None));
        }
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr initializes original on success; standard input is a valid fd.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } != 0 {
            return Ok(Self(None));
        }
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        // SAFETY: both functions receive valid termios storage and do not retain pointers.
        unsafe {
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self(Some(original)))
    }
}
impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Some(original) = &self.0 {
            // SAFETY: original was obtained from the same terminal and remains valid for this call.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
            }
        }
    }
}
pub fn terminal_size() -> Option<(u16, u16)> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: ioctl writes only to the supplied winsize storage.
    if unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
        && size.ws_row > 0
        && size.ws_col > 0
    {
        Some((size.ws_row, size.ws_col))
    } else {
        None
    }
}

/// Fresh 256-bit session marker capability; never reuse inherited session metadata.
pub fn integration_token() -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
