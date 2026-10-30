//! OS identity is captured at accept or launch, never supplied in a protocol payload.
use std::io;
use std::os::fd::AsRawFd;

mod exit;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) uid: u32,
    pub(crate) start: u64,
    pub(crate) version: u32,
    pub(crate) executable: String,
}

pub(crate) type ProcessIdentitySlot = std::sync::Arc<std::sync::Mutex<Option<ProcessIdentity>>>;

fn invalid() -> io::Error {
    io::Error::other("process identity unavailable or changed")
}

/// The one definitive answer to "is this process still there": it exited, or its
/// pid now names another process. Every other failure leaves the question open.
fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "process exited")
}

/// Sorts an OS error into the three answers a caller acts on: `NotFound` (the process
/// is gone), `PermissionDenied` (another user's process) and the rest (transient:
/// out of memory or descriptors, a busy host), which prove nothing about the process.
fn classify(error: io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(libc::ESRCH | libc::ENOENT) => gone(),
        Some(libc::EPERM | libc::EACCES) => io::Error::from(io::ErrorKind::PermissionDenied),
        _ => error,
    }
}

/// What the OS says about a process identity right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presence {
    Present,
    Gone,
    /// The OS could not answer. A decision that ends authority must not act on it.
    Unconfirmed,
}

impl ProcessIdentity {
    pub(crate) fn presence(&self) -> Presence {
        match inspect(self.pid) {
            Ok((current, _)) if current == *self => Presence::Present,
            Ok(_) => Presence::Gone,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Presence::Gone,
            Err(_) => Presence::Unconfirmed,
        }
    }

    /// Confirmed running. Use it to allow something; use `gone` to end something.
    pub(crate) fn alive(&self) -> bool {
        self.presence() == Presence::Present
    }

    /// Confirmed exited, or the pid names another process now.
    pub(crate) fn gone(&self) -> bool {
        self.presence() == Presence::Gone
    }

    pub(crate) fn contains(&self, peer: &Self) -> bool {
        self.alive() && peer.alive() && ancestry(peer).is_ok_and(|chain| chain.contains(self))
    }
}

pub(crate) fn ancestry(peer: &ProcessIdentity) -> io::Result<Vec<ProcessIdentity>> {
    let mut chain = Vec::new();
    let mut current = peer.clone();
    for _ in 0..128 {
        let (checked, parent) = inspect(current.pid)?;
        if checked != current || chain.iter().any(|p: &ProcessIdentity| p.pid == current.pid) {
            return Err(invalid());
        }
        chain.push(current.clone());
        if parent <= 1 || parent == current.pid {
            break;
        }
        current = match inspect(parent) {
            Ok((parent, _)) => parent,
            // A user's kernel can't inspect another user's process (an sshd or login
            // above the user's shell): the chain the kernel's user owns ends there.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => break,
            Err(error) => return Err(error),
        };
    }
    // Recheck the whole chain so PID reuse/reparenting during traversal fails closed.
    for pair in chain.windows(2) {
        let (checked, parent) = inspect(pair[0].pid)?;
        if checked != pair[0] || parent != pair[1].pid {
            return Err(invalid());
        }
    }
    Ok(chain)
}

pub(crate) fn holder(peer: &ProcessIdentity, pid: u32) -> io::Result<ProcessIdentity> {
    let result = ancestry(peer)?
        .into_iter()
        .find(|p| p.pid == pid && p.uid == peer.uid)
        .ok_or_else(|| io::Error::other("holder-pid must name the requester or an OS ancestor"))?;
    Ok(result)
}

#[cfg(target_os = "linux")]
pub(crate) fn inspect(pid: u32) -> io::Result<(ProcessIdentity, u32)> {
    use std::os::unix::fs::MetadataExt;
    #[cfg(test)]
    fault::check(pid)?;
    let path = format!("/proc/{pid}");
    let stat = std::fs::read_to_string(format!("{path}/stat")).map_err(classify)?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or_else(invalid)?
        .1
        .split_whitespace()
        .collect();
    match fields.first() {
        Some(&"Z" | &"X") => return Err(gone()),
        Some(_) => {}
        None => return Err(invalid()),
    }
    let parent = fields
        .get(1)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let start = fields
        .get(19)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let uid = std::fs::metadata(&path).map_err(classify)?.uid();
    // A process without an executable is exiting. Only another user's process may answer
    // PermissionDenied; the kernel user's own unreadable process is an identity the
    // kernel can't confirm.
    let executable = std::fs::read_link(format!("{path}/exe"))
        .map_err(|error| {
            let error = classify(error);
            if error.kind() == io::ErrorKind::NotFound || uid != unsafe { libc::geteuid() } {
                error
            } else {
                invalid()
            }
        })?
        .to_string_lossy()
        .into_owned();
    let after = std::fs::read_to_string(format!("{path}/stat")).map_err(classify)?;
    if after
        .rsplit_once(')')
        .ok_or_else(invalid)?
        .1
        .split_whitespace()
        .nth(19)
        != fields.get(19).copied()
    {
        return Err(gone());
    }
    Ok((
        ProcessIdentity {
            pid,
            uid,
            start,
            version: 0,
            executable,
        },
        parent,
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn peer(stream: &tokio::net::UnixStream) -> io::Result<ProcessIdentity> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&credentials) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut _,
            &mut len,
        )
    };
    if result != 0 || len as usize != std::mem::size_of_val(&credentials) || credentials.pid <= 0 {
        return Err(invalid());
    }
    let identity = inspect(credentials.pid as u32)?.0;
    if credentials.uid != unsafe { libc::geteuid() } || identity.uid != credentials.uid {
        return Err(io::Error::other(
            "Unix peer UID differs from the kernel user",
        ));
    }
    Ok(identity)
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct UniqueInfo {
    uuid: [u8; 16],
    unique: u64,
    parent_unique: u64,
    version: u32,
    parent_version: u32,
    reserved: [u64; 2],
}

#[cfg(target_os = "macos")]
pub(crate) fn inspect(pid: u32) -> io::Result<(ProcessIdentity, u32)> {
    #[cfg(test)]
    fault::check(pid)?;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let mut unique = UniqueInfo::default();
    let mut path = [0u8; 4096];
    // The libproc calls report failure with errno: ESRCH means the process is gone,
    // EPERM another user's process, anything else says nothing about the process.
    let failed = || classify(io::Error::last_os_error());
    unsafe {
        if libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            std::mem::size_of_val(&info) as i32,
        ) != std::mem::size_of_val(&info) as i32
        {
            return Err(failed());
        }
        if info.pbi_status == 5 {
            // SZOMB
            return Err(gone());
        }
        if libc::proc_pidinfo(
            pid as i32,
            17,
            0,
            &mut unique as *mut _ as *mut _,
            std::mem::size_of_val(&unique) as i32,
        ) != std::mem::size_of_val(&unique) as i32
        {
            return Err(failed());
        }
        if libc::proc_pidpath(pid as i32, path.as_mut_ptr() as *mut _, path.len() as u32) <= 0 {
            // As on Linux: an exiting process is gone, and only another user's process
            // answers PermissionDenied (libproc reports its path as unavailable with
            // whatever errno the OS version picks).
            let error = failed();
            return Err(if error.kind() == io::ErrorKind::NotFound {
                error
            } else if info.pbi_uid == unsafe { libc::geteuid() } {
                invalid()
            } else {
                io::Error::from(io::ErrorKind::PermissionDenied)
            });
        }
    }
    let executable =
        String::from_utf8_lossy(&path[..path.iter().position(|b| *b == 0).unwrap_or(path.len())])
            .into_owned();
    Ok((
        ProcessIdentity {
            pid,
            uid: info.pbi_uid,
            start: unique.unique,
            version: unique.version,
            executable,
        },
        info.pbi_ppid,
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn peer(stream: &tokio::net::UnixStream) -> io::Result<ProcessIdentity> {
    let mut uid = 0;
    let mut gid = 0;
    let mut token = [0u32; 8];
    let mut len = std::mem::size_of_val(&token) as libc::socklen_t;
    unsafe {
        if libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) != 0
            || uid != libc::geteuid()
            || libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERTOKEN,
                token.as_mut_ptr() as *mut _,
                &mut len,
            ) != 0
            || len as usize != std::mem::size_of_val(&token)
        {
            return Err(invalid());
        }
    }
    let identity = inspect(token[5])?.0;
    if identity.uid != uid || identity.version != token[7] || token[1] != uid {
        return Err(invalid());
    }
    Ok(identity)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn inspect(_: u32) -> io::Result<(ProcessIdentity, u32)> {
    Err(invalid())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn peer(_: &tokio::net::UnixStream) -> io::Result<ProcessIdentity> {
    Err(invalid())
}

/// Compare process birth in the OS clock domain. Same-tick births fail closed.
#[cfg(target_os = "linux")]
pub(crate) fn birth_cutoff() -> io::Result<u64> {
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz <= 0 || unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) } != 0 {
        return Err(invalid());
    }
    Ok(now.tv_sec as u64 * hz as u64 + now.tv_nsec as u64 * hz as u64 / 1_000_000_000)
}
#[cfg(target_os = "linux")]
pub(crate) fn born_after(peer: &ProcessIdentity, cutoff: u64) -> bool {
    peer.start > cutoff
}
#[cfg(target_os = "macos")]
// libc deprecates its binding in favor of mach2; the OS API remains supported.
#[allow(deprecated)]
pub(crate) fn birth_cutoff() -> io::Result<u64> {
    let now = unsafe { libc::mach_absolute_time() };
    if now == 0 {
        return Err(invalid());
    }
    Ok(now)
}
#[cfg(target_os = "macos")]
pub(crate) fn born_after(peer: &ProcessIdentity, cutoff: u64) -> bool {
    let mut info: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
    // Both values use mach absolute time, so wall-clock corrections cannot
    // turn an old descendant into a post-admission process.
    (unsafe {
        libc::proc_pid_rusage(
            peer.pid as i32,
            libc::RUSAGE_INFO_V0,
            (&mut info as *mut libc::rusage_info_v0).cast::<libc::rusage_info_t>(),
        )
    }) == 0
        && info.ri_proc_start_abstime > cutoff
        && peer.alive()
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn birth_cutoff() -> io::Result<u64> {
    Err(invalid())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn born_after(_: &ProcessIdentity, _: u64) -> bool {
    false
}

/// Makes `inspect` fail for one pid with an OS error, as a host out of memory or
/// descriptors does. Keyed by pid so tests running side by side do not disturb each other.
#[cfg(test)]
pub(crate) mod fault {
    use super::*;
    static FAULTS: std::sync::Mutex<Vec<(u32, i32)>> = std::sync::Mutex::new(Vec::new());

    pub(crate) struct Fault(u32);

    pub(crate) fn inject(pid: u32, errno: i32) -> Fault {
        FAULTS.lock().unwrap().push((pid, errno));
        Fault(pid)
    }

    impl Drop for Fault {
        fn drop(&mut self) {
            FAULTS.lock().unwrap().retain(|(pid, _)| *pid != self.0);
        }
    }

    pub(super) fn check(pid: u32) -> io::Result<()> {
        match FAULTS
            .lock()
            .unwrap()
            .iter()
            .find(|(faulted, _)| *faulted == pid)
        {
            Some((_, errno)) => Err(classify(io::Error::from_raw_os_error(*errno))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn peer_identity_is_os_verified_and_reused_pid_or_version_fails() {
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let identity = peer(&server).unwrap();
        assert_eq!(identity.pid, std::process::id());
        assert_eq!(identity.uid, unsafe { libc::geteuid() });
        assert!(!identity.executable.is_empty());
        assert!(identity.contains(&peer(&client).unwrap()));
        let mut reused = identity.clone();
        reused.start = reused.start.wrapping_add(1);
        assert!(!reused.alive());
        assert!(!reused.contains(&identity));
        let mut different_exec = identity.clone();
        different_exec.version = different_exec.version.wrapping_add(1);
        assert!(!different_exec.alive());
        assert!(holder(&identity, u32::MAX).is_err());
    }

    #[test]
    fn only_a_confirmed_exit_is_gone() {
        for errno in [libc::ESRCH, libc::ENOENT] {
            let error = classify(io::Error::from_raw_os_error(errno));
            assert_eq!(error.kind(), io::ErrorKind::NotFound, "errno {errno}");
        }
        for errno in [libc::EPERM, libc::EACCES] {
            let error = classify(io::Error::from_raw_os_error(errno));
            assert_eq!(
                error.kind(),
                io::ErrorKind::PermissionDenied,
                "errno {errno}"
            );
        }
        for errno in [
            libc::ENOMEM,
            libc::ENFILE,
            libc::EMFILE,
            libc::EIO,
            libc::EINTR,
        ] {
            let error = classify(io::Error::from_raw_os_error(errno));
            assert_eq!(error.raw_os_error(), Some(errno));
        }

        let mut child = std::process::Command::new("sleep")
            .arg("600")
            .spawn()
            .unwrap();
        let identity = inspect(child.id()).unwrap().0;
        assert_eq!(identity.presence(), Presence::Present);
        for errno in [libc::ENOMEM, libc::ENFILE, libc::EMFILE, libc::EIO] {
            let _fault = fault::inject(identity.pid, errno);
            assert_eq!(identity.presence(), Presence::Unconfirmed, "errno {errno}");
            assert!(!identity.alive() && !identity.gone(), "errno {errno}");
        }
        {
            let _fault = fault::inject(identity.pid, libc::ESRCH);
            assert_eq!(identity.presence(), Presence::Gone);
        }
        assert_eq!(identity.presence(), Presence::Present);
        // A zombie has exited, whether or not its parent has collected it.
        assert!(child.id() > 1);
        child.kill().unwrap();
        let started = std::time::Instant::now();
        while !identity.gone() {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.wait().unwrap();
        assert_eq!(identity.presence(), Presence::Gone);
    }

    #[test]
    fn ancestry_ends_below_another_users_process() {
        // A user's kernel started from an SSH or login session runs below a root-owned
        // process it can't inspect; the chain ends there instead of failing.
        let me = inspect(std::process::id()).unwrap().0;
        let chain = ancestry(&me).unwrap();
        assert_eq!(chain[0], me);
    }
}
