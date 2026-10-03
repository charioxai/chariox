//! OS identity is captured at accept or launch, never supplied in a protocol payload.
use std::io;
use std::os::fd::AsRawFd;

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl ProcessIdentity {
    pub(crate) fn alive(&self) -> bool {
        inspect(self.pid).is_ok_and(|(current, _)| current == *self)
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
        current = inspect(parent)?.0;
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
    let path = format!("/proc/{pid}");
    let stat = std::fs::read_to_string(format!("{path}/stat"))?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or_else(invalid)?
        .1
        .split_whitespace()
        .collect();
    if fields
        .first()
        .is_none_or(|state| *state == "Z" || *state == "X")
    {
        return Err(invalid());
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
    let uid = std::fs::metadata(&path)?.uid();
    let executable = std::fs::read_link(format!("{path}/exe"))?
        .to_string_lossy()
        .into_owned();
    let after = std::fs::read_to_string(format!("{path}/stat"))?;
    if after
        .rsplit_once(')')
        .ok_or_else(invalid)?
        .1
        .split_whitespace()
        .nth(19)
        != fields.get(19).copied()
    {
        return Err(invalid());
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
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let mut unique = UniqueInfo::default();
    let mut path = [0u8; 4096];
    unsafe {
        if libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut _,
            std::mem::size_of_val(&info) as i32) != std::mem::size_of_val(&info) as i32
            || info.pbi_status == 5 // SZOMB
            || libc::proc_pidinfo(pid as i32, 17, 0, &mut unique as *mut _ as *mut _,
                std::mem::size_of_val(&unique) as i32) != std::mem::size_of_val(&unique) as i32
            || libc::proc_pidpath(pid as i32, path.as_mut_ptr() as *mut _, path.len() as u32) <= 0
        {
            return Err(invalid());
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
pub(crate) fn birth_cutoff() -> io::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_micros() as u64)
}
#[cfg(target_os = "macos")]
pub(crate) fn born_after(peer: &ProcessIdentity, cutoff: u64) -> bool {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    (unsafe {
        libc::proc_pidinfo(
            peer.pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    }) == size
        && info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec > cutoff
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
}
