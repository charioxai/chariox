//! One narrowly authorized gid map for an enrolled pre-bwrap entry. The helper
//! never executes a caller program, maps extra gids, or gives Apps a channel.
use super::{files, model::Owner, Error, Result};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::OpenOptionsExt, net::UnixStream},
    },
};

pub(super) struct Peer {
    process: Process,
    executable: File,
    namespace: File,
    uid: u32,
    gid: u32,
}
pub(super) struct Mapping {
    child: Process,
}
impl Mapping {
    pub fn verify(&self, peer: &Peer) -> Result<()> {
        peer.process.alive()?;
        self.child.alive()?;
        if stat(&self.child.text("stat")?, self.child.pid)?.0 != peer.process.pid {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
struct Process {
    directory: File,
    pidfd: File,
    pid: i32,
    birth: u64,
}
impl Process {
    fn open(pid: i32) -> Result<Self> {
        if pid <= 1 {
            return Err(Error::Identity);
        }
        let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if descriptor < 0 {
            return Err(Error::Identity);
        }
        let pidfd = unsafe { File::from_raw_fd(descriptor as i32) };
        let directory = File::options()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(format!("/proc/{pid}"))?;
        let mut value = Self {
            directory,
            pidfd,
            pid,
            birth: 0,
        };
        value.birth = stat(&value.text("stat")?, pid)?.1;
        value.alive()?;
        Ok(value)
    }
    fn file(&self, name: &str, flags: i32) -> Result<File> {
        let name = CString::new(name).map_err(|_| Error::Identity)?;
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(Error::Identity);
        }
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
    fn text(&self, name: &str) -> Result<String> {
        let mut bytes = Vec::new();
        self.file(name, libc::O_RDONLY)?
            .take(4097)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(Error::Identity);
        }
        String::from_utf8(bytes).map_err(|_| Error::Identity)
    }
    fn alive(&self) -> Result<()> {
        let mut event = libc::pollfd {
            fd: self.pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut event, 1, 0) } != 0
            || stat(&self.text("stat")?, self.pid)?.1 != self.birth
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
impl Peer {
    pub fn capture(stream: &UnixStream) -> Result<Self> {
        let mut cred = std::mem::MaybeUninit::<libc::ucred>::zeroed();
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr().cast(),
                &mut size,
            )
        } != 0
            || size as usize != std::mem::size_of::<libc::ucred>()
        {
            return Err(Error::Identity);
        }
        let cred = unsafe { cred.assume_init() };
        let process = Process::open(cred.pid)?;
        let executable = process.file("exe", libc::O_RDONLY)?;
        let namespace = process.file("ns/user", libc::O_RDONLY)?;
        Ok(Self {
            process,
            executable,
            namespace,
            uid: cred.uid,
            gid: cred.gid,
        })
    }
}
pub(super) fn map(
    peer: &Peer,
    owner: &Owner,
    entry: &File,
    cgroup: &str,
    pid: i32,
    birth: u64,
) -> Result<Mapping> {
    if peer.uid != owner.uid || peer.gid != owner.gid || pid == peer.process.pid {
        return Err(Error::Identity);
    }
    peer.process.alive()?;
    if !same(&peer.executable, &peer.process.file("exe", libc::O_RDONLY)?)?
        || !same(
            &peer.namespace,
            &peer.process.file("ns/user", libc::O_RDONLY)?,
        )?
    {
        return Err(Error::Identity);
    }
    let child = Process::open(pid)?;
    if child.birth != birth
        || stat(&child.text("stat")?, pid)?.0 != peer.process.pid
        || child.text("cgroup")? != cgroup
        || !same(entry, &child.file("exe", libc::O_RDONLY)?)?
    {
        return Err(Error::Identity);
    }
    // Namespace ownership and its exact parent are kernel facts, not request fields.
    let status = child.text("status")?;
    if !identity_status(&status, owner.uid, owner.gid) {
        return Err(Error::Identity);
    }
    let namespace = child.file("ns/user", libc::O_RDONLY)?;
    let mut namespace_uid: libc::uid_t = 0;
    if unsafe { libc::ioctl(namespace.as_raw_fd(), 0xb704, &mut namespace_uid) } != 0
        || namespace_uid != owner.uid
    {
        return Err(Error::Identity);
    }
    let parent = unsafe { libc::ioctl(namespace.as_raw_fd(), 0xb702) }; // NS_GET_PARENT
    if parent < 0 {
        return Err(Error::Identity);
    }
    let parent = unsafe { File::from_raw_fd(parent) };
    if !same(&parent, &peer.namespace)? || same(&namespace, &peer.namespace)? {
        return Err(Error::Identity);
    }
    if !child.text("uid_map")?.is_empty()
        || !child.text("gid_map")?.is_empty()
        || child.text("setgroups")? != "allow\n"
    {
        return Err(Error::Identity);
    }
    let mut uid_mapping = child.file("uid_map", libc::O_WRONLY)?;
    let mut mapping = child.file("gid_map", libc::O_WRONLY)?;
    // A held proc file pins its selected namespace; no PID pathname is reopened
    // for the write. The namespace cannot move back to its parent (no parent
    // CAP_SYS_ADMIN), so matching before/after cannot conceal namespace reuse.
    if !same(&namespace, &child.file("ns/user", libc::O_RDONLY)?)?
        || stat(&child.text("stat")?, pid)?.0 != peer.process.pid
    {
        return Err(Error::Identity);
    }
    child.alive()?;
    peer.process.alive()?;
    let uid_bytes = format!("{} {} 1\n", owner.uid, owner.uid);
    if uid_mapping.write(uid_bytes.as_bytes())? != uid_bytes.len()
        || !single_map(&child.text("uid_map")?, owner.uid)
    {
        return Err(Error::Identity);
    }
    let bytes = format!("{} {} 1\n", owner.gid, owner.gid);
    if mapping.write(bytes.as_bytes())? != bytes.len() {
        return Err(Error::Identity);
    }
    if !single_map(&child.text("gid_map")?, owner.gid) {
        return Err(Error::Identity);
    }
    Ok(Mapping { child })
}
fn same(left: &File, right: &File) -> Result<bool> {
    Ok(files::identity(left)? == files::identity(right)?)
}
fn identity_status(text: &str, uid: u32, gid: u32) -> bool {
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
    };
    if field("TracerPid:") != Some("0") {
        return false;
    }
    for (name, id) in [("Uid:", uid), ("Gid:", gid)] {
        let Some(values) = field(name) else {
            return false;
        };
        if values.split_whitespace().collect::<Vec<_>>() != vec![id.to_string(); 4] {
            return false;
        }
    }
    true
}
fn single_map(text: &str, id: u32) -> bool {
    text.split_whitespace().collect::<Vec<_>>() == [id.to_string(), id.to_string(), "1".into()]
}
fn stat(text: &str, pid: i32) -> Result<(i32, u64)> {
    let (prefix, suffix) = text.rsplit_once(')').ok_or(Error::Identity)?;
    if prefix.split_once(' ').and_then(|v| v.0.parse::<i32>().ok()) != Some(pid) {
        return Err(Error::Identity);
    }
    let values: Vec<_> = suffix.split_whitespace().collect();
    let parent = values
        .get(1)
        .and_then(|v| v.parse().ok())
        .ok_or(Error::Identity)?;
    let birth = values
        .get(19)
        .and_then(|v| v.parse().ok())
        .ok_or(Error::Identity)?;
    Ok((parent, birth))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_handshake_rejects_versions_selected_authority_and_bad_processes() {
        use super::super::model::Request;
        let valid = r#"{"operation":"map_worker_groups_v1","pid":123,"birth":99}"#;
        assert!(serde_json::from_str::<Request>(valid)
            .unwrap()
            .validate()
            .is_ok());
        for bad in [
            valid.replace("_v1", "_v2"),
            valid.replace("123", "1"),
            valid.replace("99", "0"),
            valid.replace("99", "-1"),
            valid.replace("99}", "99,\"uid\":0}"),
            valid.replace("99}", "99,\"gid\":107}"),
            valid.replace("99}", "99,\"namespace\":7}"),
            valid.replace("99}", "99,\"lease\":\"substitute\"}"),
        ] {
            assert!(serde_json::from_str::<Request>(&bad)
                .map(|value| value.validate().is_err())
                .unwrap_or(true));
        }
    }
    #[test]
    fn mappings_admit_only_exact_single_owner_id() {
        assert!(single_map("975 975 1\n", 975));
        for value in [
            "",
            "975 975 2\n",
            "0 975 1\n",
            "975 107 1\n",
            "975 975 1\n107 107 1\n",
        ] {
            assert!(!single_map(value, 975));
        }
    }
    #[test]
    fn caller_status_refuses_wrong_ids_and_tracing() {
        let valid = "Uid: 981 981 981 981\nGid: 975 975 975 975\nTracerPid: 0\n";
        assert!(identity_status(valid, 981, 975));
        assert!(!identity_status(valid, 982, 975));
        assert!(!identity_status(valid, 981, 107));
        assert!(!identity_status(
            &valid.replace("TracerPid: 0", "TracerPid: 123"),
            981,
            975
        ));
        assert!(!identity_status(
            &valid.replace("981 981 981 981", "981 0 981 981"),
            981,
            975
        ));
    }
    #[test]
    fn process_birth_ignores_spaces_and_parentheses_in_comm() {
        let value = "123 (owned (entry)) S 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 991 0";
        assert_eq!(stat(value, 123).unwrap(), (42, 991));
        assert!(stat(value, 124).is_err());
    }
}
