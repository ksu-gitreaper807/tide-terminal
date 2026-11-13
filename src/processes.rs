//! Linux terminal-session ownership. pidfds keep shutdown signals tied to a
//! particular process even when a numeric PID has since been reused.
use std::{
    collections::BTreeMap,
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
struct Identity {
    pid: u32,
    session: u32,
    started: u64,
    state: u8,
    name: String,
}
impl Identity {
    fn read(pid: u32) -> Option<Self> {
        let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
        Self::parse(pid, &String::from_utf8_lossy(&stat))
    }
    fn parse(pid: u32, stat: &str) -> Option<Self> {
        let (head, tail) = stat.rsplit_once(") ")?;
        let fields: Vec<_> = tail.split_whitespace().collect();
        Some(Self {
            pid,
            session: fields.get(3)?.parse().ok()?,
            started: fields.get(19)?.parse().ok()?,
            state: fields.first()?.as_bytes()[0],
            name: head
                .split_once('(')?
                .1
                .chars()
                .filter(|c| !c.is_control())
                .collect(),
        })
    }
    fn same_process(&self, other: &Self) -> bool {
        self.pid == other.pid && self.started == other.started
    }
    fn live(&self) -> bool {
        !matches!(self.state, b'Z' | b'X')
    }
}
struct Process {
    identity: Identity,
    fd: OwnedFd,
}
impl Process {
    fn open(identity: Identity) -> std::io::Result<Self> {
        // SAFETY: pidfd_open takes a PID and zero flags, returning a new owned fd.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        if !Identity::read(identity.pid).is_some_and(|now| identity.same_process(&now)) {
            return Err(std::io::Error::other(
                "Process exited during ownership check",
            ));
        }
        Ok(Self { identity, fd })
    }
    fn signal(&self, signal: i32) {
        if !Identity::read(self.identity.pid).is_some_and(|now| {
            self.identity.same_process(&now) && now.session == self.identity.session
        }) {
            return;
        }
        // SAFETY: the fd identifies this process, not a reusable numeric PID.
        // A null siginfo requests an ordinary signal; zero flags are required.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
}

pub(crate) struct SessionProcesses {
    leader: Identity,
    members: BTreeMap<u32, Process>,
    scanned: Instant,
}
impl SessionProcesses {
    pub(crate) fn new(pid: u32) -> std::io::Result<Self> {
        let leader = Identity::read(pid)
            .ok_or_else(|| std::io::Error::other("Cannot inspect terminal process"))?;
        if leader.session != pid {
            return Err(std::io::Error::other(
                "Terminal process did not create an isolated session",
            ));
        }
        let process = Process::open(leader.clone())?;
        Ok(Self {
            leader,
            members: BTreeMap::from([(pid, process)]),
            scanned: Instant::now(),
        })
    }
    pub(crate) fn refresh_due(&self) -> bool {
        self.scanned.elapsed() >= Duration::from_millis(250)
    }
    pub(crate) fn refresh(&mut self) {
        self.scanned = Instant::now();
        // An unreaped leader or a verified retained member anchors the session.
        // Never rediscover a session solely from an old, reusable numeric ID.
        let anchored = Identity::read(self.leader.pid)
            .is_some_and(|now| self.leader.same_process(&now))
            || self.members.values().any(|p| {
                Identity::read(p.identity.pid).is_some_and(|now| {
                    p.identity.same_process(&now) && now.session == self.leader.pid
                })
            });
        self.members.retain(|_, p| {
            if let Some(now) = Identity::read(p.identity.pid)
                && p.identity.same_process(&now)
                && now.session == self.leader.pid
            {
                p.identity = now;
                true
            } else {
                false
            }
        });
        if !anchored {
            return;
        }
        let Ok(entries) = fs::read_dir("/proc") else {
            return;
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            else {
                continue;
            };
            if self.members.contains_key(&pid) {
                continue;
            }
            if let Some(identity) = Identity::read(pid)
                && identity.session == self.leader.pid
                && let Ok(process) = Process::open(identity)
            {
                self.members.insert(pid, process);
            }
        }
    }
    pub(crate) fn active_names(&mut self, include_leader: bool) -> Vec<String> {
        self.refresh();
        self.members
            .values()
            .filter(|p| p.identity.live() && (include_leader || p.identity.pid != self.leader.pid))
            .map(|p| format!("{} (PID {})", p.identity.name, p.identity.pid))
            .collect()
    }
    pub(crate) fn shutdown(&mut self) {
        self.refresh();
        for process in self
            .members
            .values()
            .filter(|p| p.identity.pid != self.leader.pid)
        {
            process.signal(libc::SIGHUP);
            process.signal(libc::SIGCONT);
        }
        // Let parents reap their children before asking the shell to exit.
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            self.refresh();
            if !self
                .members
                .values()
                .any(|p| p.identity.pid != self.leader.pid && p.identity.live())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for process in self
            .members
            .values()
            .filter(|p| p.identity.pid != self.leader.pid)
        {
            process.signal(libc::SIGKILL);
        }
        if let Some(root) = self.members.get(&self.leader.pid) {
            root.signal(libc::SIGHUP);
            root.signal(libc::SIGCONT);
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            self.refresh();
            if !self.members.values().any(|p| p.identity.live()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for process in self.members.values() {
            process.signal(libc::SIGKILL);
        }
    }
}

/// Observe exit without reaping, so descendants can be inventoried while their
/// session leader is still verifiably ours. portable-pty then reaps that child.
pub(crate) fn child_exited(pid: u32) -> std::io::Result<bool> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_identity_matches_kernel_session_and_rejects_reused_identity() {
        let pid = std::process::id();
        let identity = Identity::read(pid).unwrap();
        assert_eq!(identity.session, unsafe { libc::getsid(0) } as u32);
        assert!(identity.started > 0 && identity.live());
        let mut stale = identity;
        stale.started += 1;
        assert!(Process::open(stale).is_err());
    }
}
