//! Signals require a retained PID/start witness, including every group member.
#[cfg(all(test, target_os = "linux"))]
#[path = "owned_process_signals_pinned_tests.rs"]
mod pinned_tests;
#[cfg(target_os = "macos")]
use crate::runtime::bounded_process::BoundedCommandExt as _;
use std::{collections::BTreeMap, io, process::Child};

#[derive(Clone, Debug)]
struct ProcessIdentity {
    pid: u32,
    parent: u32,
    group: u32,
    session: u32,
    start: String,
    #[cfg(target_os = "linux")]
    exited: bool,
}

pub(crate) struct OwnedProcessSignals {
    root: u32,
    exclusive_session: Option<u32>,
    owned: BTreeMap<u32, String>,
}

impl OwnedProcessSignals {
    pub(crate) fn for_child(child: &Child) -> io::Result<Self> {
        Self::capture(child, false)
    }

    /// Only for a child launched with a successful setsid pre_exec hook.
    /// Retain that launch witness even after macOS clears the exited leader's session flag.
    pub(crate) fn for_session_child(child: &Child) -> io::Result<Self> {
        Self::capture(child, true)
    }

    /// A process this kernel did not just spawn (an orphan found by scanning).
    /// Its identity is only as good as the moment of this snapshot.
    pub(crate) fn for_pid(pid: u32) -> io::Result<Self> {
        Self::capture_pid(pid, false)
    }

    fn capture(child: &Child, launched_exclusive_session: bool) -> io::Result<Self> {
        Self::capture_pid(child.id(), launched_exclusive_session)
    }

    /// The caller launched this async child with a successful setsid hook.
    pub(crate) fn for_tokio_session_child(child: &tokio::process::Child) -> io::Result<Self> {
        Self::capture_pid(
            child
                .id()
                .ok_or_else(|| io::Error::other("child already reaped"))?,
            true,
        )
    }

    fn capture_pid(pid: u32, launched_exclusive_session: bool) -> io::Result<Self> {
        let rows = snapshot()?;
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(io::Error::other("invalid child PID"));
        }
        let identity = rows
            .iter()
            .find(|row| row.pid == pid)
            .ok_or_else(|| io::Error::other("child start identity unavailable"))?;
        if identity.start.is_empty() {
            return Err(io::Error::other("child birth witness unavailable"));
        }
        if launched_exclusive_session && identity.group != pid {
            return Err(io::Error::other(
                "child no longer has its launched process group",
            ));
        }
        let mut guard = Self {
            root: pid,
            exclusive_session: (launched_exclusive_session || identity.session == pid)
                .then_some(pid),
            owned: BTreeMap::from([(pid, identity.start.clone())]),
        };
        guard.refresh_from(&rows);
        Ok(guard)
    }

    fn matches(&self, row: &ProcessIdentity) -> bool {
        self.owned.get(&row.pid) == Some(&row.start)
    }

    fn refresh_from(&mut self, rows: &[ProcessIdentity]) {
        loop {
            let added: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row.pid > 1
                        && !row.start.is_empty()
                        && !self.owned.contains_key(&row.pid)
                        && (rows
                            .iter()
                            .any(|parent| parent.pid == row.parent && self.matches(parent))
                            || rows.iter().any(|root| {
                                root.pid == self.root
                                    && self.exclusive_session == Some(row.session)
                                    && row.session != 0
                                    && self.matches(root)
                            }))
                })
                .map(|row| (row.pid, row.start.clone()))
                .collect();
            if added.is_empty() {
                break;
            }
            self.owned.extend(added);
        }
    }

    pub(crate) fn refresh(&mut self) -> io::Result<()> {
        self.refresh_from(&snapshot()?);
        Ok(())
    }

    fn group_with(
        &mut self,
        rows: &[ProcessIdentity],
        send: impl FnOnce(i32, Signal) -> io::Result<()>,
    ) -> io::Result<bool> {
        self.refresh_from(rows);
        let members: Vec<_> = rows.iter().filter(|row| row.group == self.root).collect();
        if members.is_empty() {
            return Ok(false);
        }
        if self.root <= 1
            || self.root > i32::MAX as u32
            || members.iter().any(|row| !self.matches(row))
        {
            return Err(io::Error::other("unowned or reused process group member"));
        }
        send(-(self.root as i32), Signal::Kill)?;
        Ok(true)
    }

    pub(crate) fn kill_group(&mut self) -> io::Result<bool> {
        self.group_with(&snapshot()?, send_signal)
    }

    pub(crate) fn kill_owned_processes(&mut self) -> io::Result<()> {
        self.kill_owned_with(snapshot, send_process_signal)
    }

    #[cfg(unix)]
    pub(crate) fn signal_owned_processes(&mut self, signal: i32) -> io::Result<()> {
        let rows = snapshot()?;
        self.refresh_from(&rows);
        for pid in self.parents_first(&rows) {
            if let Some(row) = snapshot()?
                .iter()
                .find(|row| row.pid == pid && self.matches(row))
            {
                send_process_signal_with(row, signal)?;
            }
        }
        Ok(())
    }

    /// Freezes the whole tree before ending any member: a waiting parent whose child is killed
    /// first would otherwise resume and run its next command before its own signal arrives.
    /// Frozen parents cannot resume, so descendants end first and the root, whose exit a caller
    /// waits for, ends last.
    fn kill_owned_with(
        &mut self,
        mut inspect: impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        mut send: impl FnMut(&ProcessIdentity, Signal) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut stopped = std::collections::BTreeSet::new();
        loop {
            let rows = inspect()?;
            self.refresh_from(&rows);
            // A member forked before its parent froze appears in the next snapshot.
            let pending: Vec<_> = self
                .parents_first(&rows)
                .into_iter()
                .filter(|pid| !stopped.contains(pid))
                .collect();
            if pending.is_empty() {
                break;
            }
            for pid in pending {
                self.signal_current(pid, Signal::Stop, &mut inspect, &mut send)?;
                stopped.insert(pid);
            }
        }
        let rows = inspect()?;
        for pid in self.parents_first(&rows).into_iter().rev() {
            self.signal_current(pid, Signal::Kill, &mut inspect, &mut send)?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn all_exited(&self) -> io::Result<bool> {
        for (pid, start) in &self.owned {
            let row = witnessed_row(*pid)?;
            if !witnessed_exit(start, row)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn pin_owned_processes_into(
        &mut self,
        handles: &mut OwnedProcessHandles,
    ) -> io::Result<()> {
        let rows = snapshot()?;
        self.refresh_from(&rows);
        for row in &rows {
            if row.start.is_empty()
                && self.owned.contains_key(&row.pid)
                && !handles.0.contains_key(&row.pid)
            {
                return Err(io::Error::other(
                    "owned process identity is unreadable before pinning",
                ));
            }
            if !self.matches(row) || handles.0.contains_key(&row.pid) {
                continue;
            }
            if let Some(handle) = pin_process(row)? {
                // Publish each verified handle immediately. A subsequent pin
                // failure must retain both old and newly acquired ownership.
                handles.0.insert(row.pid, handle);
            }
        }
        for (pid, start) in &self.owned {
            if !handles.0.contains_key(pid) && !witnessed_exit(start, witnessed_row(*pid)?)? {
                return Err(io::Error::other("live owned process could not be pinned"));
            }
        }
        Ok(())
    }

    /// Owned members still alive, every parent before its descendants.
    fn parents_first(&self, rows: &[ProcessIdentity]) -> Vec<u32> {
        let alive: BTreeMap<u32, u32> = rows
            .iter()
            .filter(|row| self.matches(row))
            .map(|row| (row.pid, row.parent))
            .collect();
        let depth = |mut pid: u32| {
            let mut depth = 0;
            while let Some(parent) = alive.get(&pid).copied().filter(|p| alive.contains_key(p)) {
                if depth > alive.len() {
                    break;
                }
                depth += 1;
                pid = parent;
            }
            depth
        };
        let mut order: Vec<_> = alive.keys().map(|pid| (depth(*pid), *pid)).collect();
        order.sort_unstable();
        order.into_iter().map(|(_, pid)| pid).collect()
    }

    /// Carries the verified birth to the signal seam, where Linux pins it before sending.
    fn signal_current(
        &self,
        pid: u32,
        signal: Signal,
        inspect: &mut impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        send: &mut impl FnMut(&ProcessIdentity, Signal) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(row) = inspect()?
            .iter()
            .find(|row| row.pid == pid && self.matches(row))
        {
            send(row, signal)?;
        }
        Ok(())
    }

    /// The launched leader's PID and birth, for a record that outlives this kernel.
    pub(crate) fn root_birth(&self) -> (u32, String) {
        (
            self.root,
            self.owned.get(&self.root).cloned().unwrap_or_default(),
        )
    }

    /// Ends what remains of a tree an earlier kernel launched in its own session with `claim`
    /// in its environment. Ok(true): nothing of it is left. Ok(false): its members were just
    /// signalled; ask again. An error means some member could not be inspected, so it may live.
    pub(crate) fn end_claimed_tree(claim: &str, leader: Option<(u32, &str)>) -> io::Result<bool> {
        Self::end_claimed_tree_with(
            claim,
            leader,
            snapshot,
            read_environment,
            send_process_signal,
        )
    }

    fn end_claimed_tree_with(
        claim: &str,
        leader: Option<(u32, &str)>,
        mut inspect: impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        environment: impl Fn(u32) -> io::Result<Option<Vec<u8>>>,
        send: impl FnMut(&ProcessIdentity, Signal) -> io::Result<()>,
    ) -> io::Result<bool> {
        if claim.is_empty() {
            return Err(io::Error::other("the launch claim is missing"));
        }
        let marker = format!("{CLAIM_VARIABLE}={claim}").into_bytes();
        let rows = inspect()?;
        // The saved leader is a witness of its own: the same PID with the same birth is the
        // script's session leader, so its tree is known without reading any environment.
        let live_leader = leader
            .filter(|(pid, birth)| *pid > 1 && !birth.is_empty())
            .and_then(|(pid, birth)| rows.iter().find(|row| row.pid == pid && row.start == birth));
        let mut owned = BTreeMap::new();
        for row in rows.iter().filter(|row| row.pid > 1) {
            if live_leader.is_some_and(|leader| leader.pid == row.pid) {
                owned.insert(row.pid, row.start.clone());
                continue;
            }
            let named_by_launch = leader.is_some_and(|(pid, birth)| {
                (row.pid == pid && row.start == birth) || (row.session == pid && pid > 1)
            });
            let marked = match environment(row.pid)? {
                Some(bytes) => bytes.split(|byte| *byte == 0).any(|entry| entry == marker),
                // The leader's session is ended through the leader below.
                None if named_by_launch && live_leader.is_none() => {
                    return Err(io::Error::other("an earlier Setup process is unreadable"))
                }
                None => false,
            };
            if !marked {
                continue;
            }
            if row.start.is_empty() {
                return Err(io::Error::other("an earlier Setup process has no birth"));
            }
            owned.insert(row.pid, row.start.clone());
        }
        let Some(root) = live_leader
            .map(|leader| leader.pid)
            .or_else(|| owned.keys().next().copied())
        else {
            return Ok(true);
        };
        let mut guard = Self {
            root,
            exclusive_session: live_leader
                .filter(|leader| leader.session == leader.pid)
                .map(|leader| leader.pid),
            owned,
        };
        guard.kill_owned_with(inspect, send)?;
        Ok(false)
    }

    pub(crate) fn kill_child(&mut self) -> io::Result<bool> {
        let rows = snapshot()?;
        let Some(row) = rows.iter().find(|row| row.pid == self.root) else {
            return Ok(false);
        };
        if !self.matches(row) {
            return Err(io::Error::other("child PID was reused"));
        }
        send_process_signal(row, Signal::Kill)?;
        Ok(true)
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct OwnedProcessHandles(BTreeMap<u32, std::os::fd::OwnedFd>);

#[cfg(target_os = "linux")]
impl OwnedProcessHandles {
    pub(crate) fn signal(&self, signal: i32) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        for (pid, handle) in &self.0 {
            if *pid <= 1 {
                return Err(io::Error::other("reserved child signal target"));
            }
            send_handle_signal(handle.as_raw_fd(), signal)?;
        }
        Ok(())
    }

    pub(crate) fn all_exited(&self) -> bool {
        use std::os::fd::AsRawFd;
        self.0.values().all(|handle| {
            let mut fd = libc::pollfd {
                fd: handle.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe { libc::poll(&mut fd, 1, 0) == 1 }
        })
    }
}

#[cfg(target_os = "linux")]
fn pin_process(row: &ProcessIdentity) -> io::Result<Option<std::os::fd::OwnedFd>> {
    use std::os::fd::FromRawFd;
    if row.pid <= 1 || row.pid > i32::MAX as u32 || row.start.is_empty() {
        return Err(io::Error::other("invalid process birth witness"));
    }
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, row.pid, 0) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let handle = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) };
    // Match the birth after opening. The verified handle keeps this target
    // stable if it exits and its numeric PID is reused before signal delivery.
    if witnessed_exit(&row.start, witnessed_row(row.pid)?)? {
        return Ok(None);
    }
    Ok(Some(handle))
}

#[cfg(target_os = "linux")]
fn send_handle_signal(fd: std::os::fd::RawFd, signal: i32) -> io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd,
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result >= 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

fn send_process_signal(row: &ProcessIdentity, signal: Signal) -> io::Result<()> {
    #[cfg(unix)]
    {
        send_process_signal_with(
            row,
            match signal {
                Signal::Stop => libc::SIGSTOP,
                Signal::Kill => libc::SIGKILL,
            },
        )
    }
    #[cfg(not(unix))]
    {
        send_signal(row.pid as i32, signal)
    }
}

#[cfg(unix)]
fn send_process_signal_with(row: &ProcessIdentity, signal: i32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        if let Some(handle) = pin_process(row)? {
            send_handle_signal(handle.as_raw_fd(), signal)?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        send_signal_with(row.pid as i32, signal)
    }
}

/// Names the run a process belongs to; every descendant inherits it.
pub(crate) const CLAIM_VARIABLE: &str = "CHARIOX_SETUP_CLAIM";

/// Ok(None): the process cannot be read (another user's or hidden), so it proves nothing.
#[cfg(target_os = "linux")]
fn read_environment(pid: u32) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            Ok(Some(Vec::new()))
        }
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)) => Ok(None),
        Err(error) => Err(error),
    }
}
/// The environment comes from the process arguments the kernel keeps; another user's process
/// reads as unreadable and a process that is gone as empty, like the Linux file.
#[cfg(target_os = "macos")]
fn read_environment(pid: u32) -> io::Result<Option<Vec<u8>>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 0;
    let mut buffer = Vec::new();
    // The first call reports the size, the second fills the buffer.
    for fill in [false, true] {
        let target = if fill {
            buffer.resize(size, 0u8);
            buffer.as_mut_ptr().cast()
        } else {
            std::ptr::null_mut()
        };
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                target,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return procargs2_failure(io::Error::last_os_error(), || process_exists(pid));
        }
    }
    buffer.truncate(size);
    procargs2_environment(&buffer).map(Some)
}
/// Signal 0 delivers nothing: it only reports whether the process still exists.
#[cfg(target_os = "macos")]
fn process_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    if pid <= 1 {
        return true;
    }
    // SAFETY: signal 0 is never delivered; `pid` is a real process id (above 1).
    let delivered = unsafe { libc::kill(pid, 0) } == 0;
    delivered || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
/// What a failed `KERN_PROCARGS2` read says about the process. macOS answers EINVAL both for a
/// process that has just exited and for one it will not describe, so EINVAL means gone only when
/// the process no longer exists; a zombie still exists and stays unreadable.
#[cfg(any(target_os = "macos", test))]
fn procargs2_failure(
    error: io::Error,
    process_exists: impl FnOnce() -> bool,
) -> io::Result<Option<Vec<u8>>> {
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(Some(Vec::new())),
        Some(libc::EINVAL) if !process_exists() => Ok(Some(Vec::new())),
        Some(libc::EPERM | libc::EACCES | libc::EINVAL) => Ok(None),
        _ => Err(error),
    }
}
/// No environment is readable here: only the launch witnesses can show the tree is gone.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_environment(_: u32) -> io::Result<Option<Vec<u8>>> {
    Ok(None)
}

/// The environment of a `KERN_PROCARGS2` buffer, as NUL-ended entries like `/proc/<pid>/environ`.
/// The buffer is the argument count (a native `i32`), the executable path, NUL padding, that
/// many arguments, then the environment strings. A last entry cut short by the kernel's size
/// limit is dropped; a buffer that ends before the environment starts is an error.
#[cfg(any(target_os = "macos", test))]
fn procargs2_environment(buffer: &[u8]) -> io::Result<Vec<u8>> {
    let malformed = || io::Error::other("the process arguments are not readable");
    let (count, rest) = buffer.split_first_chunk::<4>().ok_or_else(malformed)?;
    let count = usize::try_from(i32::from_ne_bytes(*count)).map_err(|_| malformed())?;
    // The executable path, then the padding that follows it.
    let path_end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(malformed)?;
    let mut rest = &rest[path_end..];
    while let [0, tail @ ..] = rest {
        rest = tail;
    }
    for _ in 0..count {
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(malformed)?;
        rest = &rest[end + 1..];
    }
    let mut environment = Vec::new();
    for entry in rest.split_inclusive(|byte| *byte == 0) {
        match entry {
            [0] => break,
            [.., 0] => environment.extend_from_slice(entry),
            _ => break,
        }
    }
    Ok(environment)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Signal {
    Stop,
    Kill,
}

#[cfg(unix)]
fn send_signal(pid: i32, signal: Signal) -> io::Result<()> {
    let signal = match signal {
        Signal::Stop => libc::SIGSTOP,
        Signal::Kill => libc::SIGKILL,
    };
    send_signal_with(pid, signal)
}

#[cfg(unix)]
fn send_signal_with(pid: i32, signal: i32) -> io::Result<()> {
    // MP-11: reject system-wide targets even if a future caller loses its witness.
    if pid == i32::MIN || (-1..=1).contains(&pid) {
        return Err(io::Error::other("invalid process signal target"));
    }
    // Identity and complete group membership are checked immediately before this sole seam.
    if matches!(pid, -1..=1) {
        return Err(io::Error::other("reserved signal target"));
    }
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}
#[cfg(not(unix))]
fn send_signal(_: i32, _: Signal) -> io::Result<()> {
    Err(io::Error::other("owned signals unsupported"))
}

#[cfg(target_os = "linux")]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    let mut rows = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Some(row) = linux_row(pid, std::fs::read(entry.path().join("stat")))? {
            rows.push(row);
        }
    }
    Ok(rows)
}

#[cfg(target_os = "linux")]
fn linux_row(pid: u32, stat: io::Result<Vec<u8>>) -> io::Result<Option<ProcessIdentity>> {
    let bytes = match stat {
        Ok(bytes) => bytes,
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            return Ok(None)
        }
        // hidepid or another user's process: its group and session stay
        // visible, its start identity does not, so it is never ours.
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)) => {
            return Ok(unreadable_row(pid))
        }
        Err(error) => return Err(error),
    };
    // The process name is arbitrary bytes; the numbers follow its last ')'.
    let Some(end) = bytes.iter().rposition(|byte| *byte == b')') else {
        return Err(io::Error::other("invalid process identity"));
    };
    let tail = std::str::from_utf8(&bytes[end + 1..]).map_err(io::Error::other)?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    if fields.len() < 20 {
        return Err(io::Error::other("short process identity"));
    }
    // A row whose ids are not numbers (a process that is exiting, or one in another PID
    // namespace) is not one of ours, like a row that has vanished: it must not fail the
    // snapshot of every process on the host.
    let (Ok(parent), Ok(group), Ok(session)) =
        (fields[1].parse(), fields[2].parse(), fields[3].parse())
    else {
        return Ok(None);
    };
    Ok(Some(ProcessIdentity {
        pid,
        parent,
        group,
        session,
        start: fields[19].into(),
        exited: matches!(fields[0], "Z" | "X"),
    }))
}

#[cfg(target_os = "linux")]
fn witnessed_row(pid: u32) -> io::Result<Option<ProcessIdentity>> {
    match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(bytes) => linux_row(pid, Ok(bytes)),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn witnessed_exit(start: &str, row: Option<ProcessIdentity>) -> io::Result<bool> {
    match row {
        None => Ok(true),
        Some(row) if row.start.is_empty() => {
            Err(io::Error::other("owned process identity is unreadable"))
        }
        Some(row) => Ok(row.start != start || row.exited),
    }
}

#[cfg(target_os = "linux")]
fn unreadable_row(pid: u32) -> Option<ProcessIdentity> {
    let id = i32::try_from(pid).ok()?;
    let (group, session) = unsafe { (libc::getpgid(id), libc::getsid(id)) };
    (group > 0 && session > 0).then(|| ProcessIdentity {
        pid,
        parent: 0,
        group: group as u32,
        session: session as u32,
        start: String::new(),
        exited: false,
    })
}
#[cfg(target_os = "macos")]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    let result = crate::process_spawn::command("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .env("LC_ALL", "C")
        .bounded_output()?;
    if !result.status.success() {
        return Err(io::Error::other("process metadata unavailable"));
    }
    let mut rows = Vec::new();
    for line in String::from_utf8(result.stdout)
        .map_err(io::Error::other)?
        .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(io::Error::other("invalid process metadata"));
        }
        let parse = |field: &str| {
            field
                .parse::<u32>()
                .map_err(|_| io::Error::other("invalid process ID"))
        };
        let pid = parse(fields[0])?;
        if pid == 0 || pid > i32::MAX as u32 {
            continue;
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // arg=1 includes zombie metadata: an unreaped leader still pins its PID/birth.
        let read = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if read != size {
            let error = io::Error::last_os_error();
            if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            if read == 0 && error.raw_os_error() == Some(libc::EPERM) {
                // Preserve visible foreign group membership for rejection, never admission.
                rows.push(ProcessIdentity {
                    pid,
                    parent: parse(fields[1])?,
                    group: parse(fields[2])?,
                    session: 0,
                    start: String::new(),
                });
                continue;
            }
            return Err(io::Error::other("incomplete native process metadata"));
        }
        if info.pbi_pid != pid || info.pbi_start_tvsec == 0 {
            return Err(io::Error::other("invalid native process birth"));
        }
        let session = macos_session_id(pid, info.pbi_flags, |pid| {
            let session = unsafe { libc::getsid(pid as i32) };
            if session >= 0 {
                return Ok(session as u32);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(0)
            } else {
                Err(error)
            }
        })?;
        rows.push(ProcessIdentity {
            pid,
            parent: info.pbi_ppid,
            group: info.pbi_pgid,
            session,
            start: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
        });
    }
    Ok(rows)
}

#[cfg(any(target_os = "macos", test))]
fn macos_session_id(
    pid: u32,
    flags: u32,
    session_of: impl Fn(u32) -> io::Result<u32>,
) -> io::Result<u32> {
    // Public Darwin proc_info.h PROC_FLAG_SLEADER. Exited leaders lose this flag;
    // their exclusive-session launch witness is retained separately by the guard.
    const PROC_FLAG_SLEADER: u32 = 0x20;
    if flags & PROC_FLAG_SLEADER != 0 {
        Ok(pid)
    } else {
        session_of(pid)
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    Err(io::Error::other("owned process inspection unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(pid: u32, parent: u32, group: u32, start: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            parent,
            group,
            session: 0,
            start: start.into(),
            #[cfg(target_os = "linux")]
            exited: false,
        }
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn witnessed_exit_distinguishes_zombies_reuse_and_unknown_identity() {
        assert!(!witnessed_exit("birth", Some(row(42, 1, 42, "birth"))).unwrap());
        let mut zombie = row(42, 1, 42, "birth");
        zombie.exited = true;
        assert!(witnessed_exit("birth", Some(zombie)).unwrap());
        assert!(witnessed_exit("birth", Some(row(42, 1, 42, "new-birth"))).unwrap());
        assert!(witnessed_exit("birth", None).unwrap());
        assert!(witnessed_exit("birth", Some(row(42, 1, 42, ""))).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn repeated_pins_retain_the_same_process_handles() {
        use std::os::fd::AsRawFd;
        let mut child = crate::process_spawn::command("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut owner = OwnedProcessSignals::for_child(&child).unwrap();
        let mut handles = OwnedProcessHandles::default();
        owner.pin_owned_processes_into(&mut handles).unwrap();
        let before: Vec<_> = handles
            .0
            .iter()
            .map(|(pid, handle)| (*pid, handle.as_raw_fd()))
            .collect();
        let repinned = owner.pin_owned_processes_into(&mut handles);
        let after: Vec<_> = handles
            .0
            .iter()
            .map(|(pid, handle)| (*pid, handle.as_raw_fd()))
            .collect();
        crate::runtime::child_shutdown::stop(&mut child, "repin_fixture").unwrap();
        assert!(repinned.is_ok());
        assert!(!before.is_empty());
        assert_eq!(before, after);
    }

    /// A process can only rename itself, so a shell does it and then waits.
    #[cfg(target_os = "linux")]
    fn spawn_shell_named_invalidly() -> std::process::Child {
        use std::process::Stdio;
        let child = std::process::Command::new("sh")
            .args([
                "-c",
                r#"printf '\377\376' > /proc/$$/comm; read line; true"#,
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let stat = format!("/proc/{}/stat", child.id());
        while !std::fs::read(&stat).unwrap().contains(&0xff) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_foreign_process_with_a_non_utf8_name_does_not_block_a_witness() {
        let mut foreign = spawn_shell_named_invalidly();
        let mut own = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let witness = OwnedProcessSignals::for_child(&own);
        let seen = snapshot();
        let _ = foreign.kill();
        let _ = foreign.wait();
        let _ = own.kill();
        let _ = own.wait();
        assert!(witness.is_ok(), "witness failed: {:?}", witness.err());
        assert!(seen.unwrap().iter().any(|row| row.pid == foreign.id()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_row_with_non_numeric_ids_is_skipped_not_an_error() {
        let stat =
            |ids: &str| format!("77 (x) S {ids} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 5000 0").into_bytes();
        assert!(linux_row(77, Ok(stat("1 1 1"))).unwrap().is_some());
        for ids in ["1 -1 1", "1 1 -1", "-1 1 1"] {
            assert!(
                linux_row(77, Ok(stat(ids))).unwrap().is_none(),
                "a row with ids `{ids}` failed the snapshot"
            );
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn an_unreadable_process_still_counts_as_a_group_member_but_is_never_owned() {
        let denied = || Err(io::Error::from_raw_os_error(libc::EACCES));
        let own = std::process::id();
        let hidden = linux_row(own, denied())
            .unwrap()
            .expect("row for a live process");
        assert!(hidden.start.is_empty());
        assert_eq!(hidden.group, unsafe { libc::getpgrp() } as u32);
        let mut guard = OwnedProcessSignals {
            root: hidden.group,
            exclusive_session: None,
            owned: BTreeMap::from([(hidden.group, "root".into())]),
        };
        let leader = row(hidden.group, 1, hidden.group, "root");
        assert!(guard
            .group_with(&[leader, hidden], |_, _| panic!(
                "unreadable member signalled"
            ))
            .is_err());
    }

    #[test]
    fn mp11_review_macos_session_witness_admits_fast_reparented_child() {
        assert_eq!(
            macos_session_id(41, 0x20, |_| panic!("leader does not need live getsid")).unwrap(),
            41
        );
        let mut leader = row(41, 10, 41, "precise-leader-birth");
        leader.session = macos_session_id(41, 0, |_| Ok(0)).unwrap(); // zombie getsid = ESRCH
        let mut orphan = row(42, 1, 41, "precise-child-birth");
        orphan.session = macos_session_id(42, 0, |_| Ok(41)).unwrap();
        let rows = [leader, orphan];
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: Some(41), // retained setsid launch witness
            owned: BTreeMap::from([(41, rows[0].start.clone())]),
        };
        assert!(guard
            .group_with(&rows, |pid, _| {
                assert_eq!(pid, -41);
                Ok(())
            })
            .unwrap());
        let mut foreign = row(43, 1, 41, "foreign-birth");
        foreign.session = 99;
        assert!(guard
            .group_with(&[rows[0].clone(), foreign], |_, _| panic!(
                "foreign session signalled"
            ))
            .is_err());
        let mut lost = OwnedProcessSignals {
            root: 41,
            exclusive_session: Some(41),
            owned: BTreeMap::from([(41, rows[0].start.clone())]),
        };
        assert!(lost
            .group_with(&rows[1..], |_, _| panic!(
                "missing leader witness signalled"
            ))
            .is_err());
        let mut replacement = rows[0].clone();
        replacement.start = "replacement-leader".into();
        assert!(lost
            .group_with(&[replacement, rows[1].clone()], |_, _| panic!(
                "reused leader admitted children"
            ))
            .is_err());
    }

    #[test]
    fn mp11_review_failed_group_cleanup_signals_only_current_owned_births() {
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([
                (41, "root".into()),
                (43, "child".into()),
                (44, "original".into()),
            ]),
        };
        let rows = vec![
            row(41, 1, 41, "root"),
            row(42, 1, 41, "foreign"),
            row(43, 1, 41, "child"),
            row(44, 1, 41, "reused"),
        ];
        assert!(guard
            .group_with(&rows, |_, _| panic!("mixed group signalled"))
            .is_err());
        let mut signalled = Vec::new();
        guard
            .kill_owned_with(
                || Ok(rows.clone()),
                |pid, signal| {
                    signalled.push((pid.pid as i32, signal));
                    Ok(())
                },
            )
            .unwrap();
        use Signal::{Kill, Stop};
        assert_eq!(signalled, [(41, Stop), (43, Stop), (43, Kill), (41, Kill)]);
    }

    #[test]
    fn an_earlier_kernels_claimed_tree_is_frozen_then_ended_and_unreadable_members_keep_it_alive() {
        let mut leader = row(41, 1, 41, "leader");
        leader.session = 41;
        let mut child = row(42, 41, 41, "child");
        child.session = 41;
        let mut reused = row(43, 1, 43, "reused");
        reused.session = 43;
        let rows = vec![leader, child, reused];
        let environment = |pid: u32| {
            Ok(Some(match pid {
                41 | 42 => b"PATH=/bin\0CHARIOX_SETUP_CLAIM=abc\0".to_vec(),
                _ => b"CHARIOX_SETUP_CLAIM=abcd\0".to_vec(),
            }))
        };
        let mut sent = Vec::new();
        let ended = OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((41, "leader")),
            || Ok(rows.clone()),
            environment,
            |pid, signal| {
                sent.push((pid.pid as i32, signal));
                Ok(())
            },
        )
        .unwrap();
        use Signal::{Kill, Stop};
        assert!(!ended);
        assert_eq!(sent, [(41, Stop), (42, Stop), (42, Kill), (41, Kill)]);
        // Nothing carries the claim and the saved leader birth names another process.
        assert!(OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((43, "earlier")),
            || Ok(vec![rows[2].clone()]),
            environment,
            |_, _| panic!("a reused PID was signalled"),
        )
        .unwrap());
        // With the leader gone, a member of its session that cannot be read may still be the script.
        assert!(OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((41, "leader")),
            || Ok(vec![rows[1].clone(), rows[2].clone()]),
            |pid| Ok((pid != 42).then(Vec::new)),
            |_, _| panic!("an unproven tree was signalled"),
        )
        .is_err());
    }

    /// The platform shows no environment (an unreadable process), so only the saved leader
    /// names the tree; the orphan stays alive until the signals reach it.
    #[test]
    fn a_leader_with_its_saved_birth_ends_its_session_without_any_environment() {
        use std::{cell::RefCell, collections::BTreeSet};
        let mut leader = row(41, 1, 41, "leader");
        leader.session = 41;
        let mut child = row(42, 41, 41, "child");
        child.session = 41;
        // Detached from the leader's parent chain, still in its session.
        let mut stray = row(44, 1, 44, "stray");
        stray.session = 41;
        let mut other = row(45, 1, 45, "other");
        other.session = 45;
        let alive = RefCell::new(vec![leader, child, stray, other]);
        let sent = RefCell::new(Vec::new());
        let unreadable = |_: u32| Ok(None);
        let end = |leader_birth: &str| {
            OwnedProcessSignals::end_claimed_tree_with(
                "abc",
                Some((41, leader_birth)),
                || Ok(alive.borrow().clone()),
                unreadable,
                |pid, signal| {
                    sent.borrow_mut().push((pid.pid as i32, signal));
                    if signal == Signal::Kill {
                        alive.borrow_mut().retain(|row| row.pid != pid.pid);
                    }
                    Ok(())
                },
            )
        };
        // A reused PID: another birth is never signalled.
        assert!(end("earlier")
            .unwrap_err()
            .to_string()
            .contains("unreadable"));
        assert!(sent.borrow().is_empty());
        assert_eq!(alive.borrow().len(), 4);
        // The saved birth: the whole session is frozen, then ended, parents first.
        assert!(!end("leader").unwrap());
        use Signal::{Kill, Stop};
        let sent = sent.borrow().clone();
        assert_eq!(sent.iter().filter(|(_, signal)| *signal == Stop).count(), 3);
        assert_eq!(
            sent.iter()
                .filter(|(_, signal)| *signal == Kill)
                .map(|(pid, _)| *pid)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([41, 42, 44])
        );
        assert!(sent[..3].iter().all(|(_, signal)| *signal == Stop));
        assert!(sent.iter().all(|(pid, _)| *pid != 45));
        // Asked again, nothing of it is left.
        assert!(end("leader").unwrap());
    }

    #[test]
    fn procargs2_failure_reads_a_vanished_process_as_gone() {
        let gone = || false;
        let errno = |code| io::Error::from_raw_os_error(code);
        assert_eq!(
            procargs2_failure(errno(libc::ESRCH), || unreachable!()).unwrap(),
            Some(Vec::new())
        );
        // EINVAL with no process behind it is a process that vanished.
        assert_eq!(
            procargs2_failure(errno(libc::EINVAL), gone).unwrap(),
            Some(Vec::new())
        );
        // EINVAL for a process that still exists proves nothing.
        assert_eq!(
            procargs2_failure(errno(libc::EINVAL), || true).unwrap(),
            None
        );
        for code in [libc::EPERM, libc::EACCES] {
            assert_eq!(
                procargs2_failure(errno(code), || unreachable!()).unwrap(),
                None
            );
        }
        assert!(procargs2_failure(errno(libc::EIO), || unreachable!()).is_err());
    }

    #[test]
    fn procargs2_environment_reads_the_entries_after_the_arguments() {
        let buffer = |count: i32, body: &[u8]| {
            let mut bytes = count.to_ne_bytes().to_vec();
            bytes.extend_from_slice(body);
            bytes
        };
        let body = b"/bin/sh\0\0\0\0sh\0-c\0A=1\0CHARIOX_SETUP_CLAIM=abc\0\0\0";
        assert_eq!(
            procargs2_environment(&buffer(2, body)).unwrap(),
            b"A=1\0CHARIOX_SETUP_CLAIM=abc\0"
        );
        // No environment at all.
        assert!(procargs2_environment(&buffer(1, b"/bin/sh\0\0sh\0"))
            .unwrap()
            .is_empty());
        // The kernel's size limit cut the last entry: it is dropped, never half-read.
        assert_eq!(
            procargs2_environment(&buffer(1, b"/bin/sh\0\0sh\0A=1\0CHARIOX_SETUP_CL")).unwrap(),
            b"A=1\0"
        );
        // Bytes that are not UTF-8 stay as they are.
        assert_eq!(
            procargs2_environment(&buffer(1, b"/bin/sh\0\0sh\0K=\xff\xfe\0")).unwrap(),
            b"K=\xff\xfe\0"
        );
        // More arguments announced than present, a buffer cut inside the path or the count, a negative count.
        assert!(procargs2_environment(&buffer(5, b"/bin/sh\0\0sh\0A=1\0")).is_err());
        assert!(procargs2_environment(&buffer(1, b"/bin/sh")).is_err());
        assert!(procargs2_environment(&[1, 0]).is_err());
        assert!(procargs2_environment(&buffer(-1, b"/bin/sh\0\0sh\0")).is_err());
        assert!(procargs2_environment(&buffer(i32::MAX, b"/bin/sh\0\0sh\0")).is_err());
        // A very large environment is read whole.
        let mut large = b"/bin/sh\0\0sh\0".to_vec();
        for index in 0..20_000 {
            large.extend_from_slice(format!("V{index}=x\0").as_bytes());
        }
        let read = procargs2_environment(&buffer(1, &large)).unwrap();
        assert_eq!(read.iter().filter(|byte| **byte == 0).count(), 20_000);
    }

    #[test]
    fn mp11_signal_guard_rejects_reserved_and_overflowed_groups_before_io() {
        for root in [0, 1, u32::MAX] {
            let rows = [row(root, 9, root, "birth")];
            let mut guard = OwnedProcessSignals {
                root,
                exclusive_session: None,
                owned: BTreeMap::from([(root, "birth".into())]),
            };
            assert!(guard
                .group_with(&rows, |_, _| panic!("invalid group signalled"))
                .is_err());
        }
    }

    #[test]
    fn mp11_cleanup_retains_detached_descendants_after_a_group_signal() {
        let rows = vec![
            row(41, 9, 41, "root"),
            row(45, 41, 45, "detached"),
            row(99, 1, 99, "foreign"),
        ];
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([(41, "root".into())]),
        };
        let mut sent = Vec::new();
        guard
            .group_with(&rows, |pid, signal| {
                sent.push((pid, signal));
                Ok(())
            })
            .unwrap();
        guard
            .kill_owned_with(
                || Ok(rows.clone()),
                |pid, signal| {
                    sent.push((pid.pid as i32, signal));
                    Ok(())
                },
            )
            .unwrap();
        use Signal::{Kill, Stop};
        assert_eq!(
            sent,
            [(-41, Kill), (41, Stop), (45, Stop), (45, Kill), (41, Kill)]
        );
    }

    #[test]
    fn mp11_signal_guard_refuses_reused_and_foreign_members() {
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([(41, "original".into())]),
        };
        let mut sends = 0;
        assert!(guard
            .group_with(&[row(41, 1, 41, "reused")], |_, _| {
                sends += 1;
                Ok(())
            })
            .is_err());
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(42, 1, 41, "foreign")],
                |_, _| {
                    sends += 1;
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(sends, 0);
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(43, 41, 41, "child")],
                |pid, _| {
                    assert_eq!(pid, -41);
                    sends += 1;
                    Ok(())
                }
            )
            .unwrap());
        assert_eq!(sends, 1);
        // Once reparented, only the recorded child birth is authorized.
        assert!(guard
            .group_with(&[row(43, 1, 41, "child")], |_, _| Ok(()))
            .unwrap());
        assert!(guard
            .group_with(&[row(43, 1, 41, "replacement")], |_, _| panic!(
                "reused child signalled"
            ))
            .is_err());
    }
}

#[cfg(all(test, unix))]
#[test]
fn mp11_signal_seam_rejects_system_wide_targets_without_signalling() {
    for pid in [i32::MIN, -1, 0, 1] {
        assert_eq!(
            send_signal(pid, Signal::Kill).unwrap_err().to_string(),
            "invalid process signal target"
        );
    }
}
