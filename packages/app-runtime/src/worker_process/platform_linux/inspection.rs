//! Independent host observations of the exact worker before native Continue.
use super::{policy, Result, WorkerError};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

const NAMESPACES: [&str; 7] = ["mnt", "user", "pid", "net", "ipc", "uts", "cgroup"];
pub(super) struct Observer {
    namespaces: Vec<File>,
    executable: File,
    uid: u32,
    gid: u32,
}
impl Observer {
    pub fn new(executable: File) -> Result<Self> {
        let namespaces = NAMESPACES
            .iter()
            .map(|name| {
                File::open(format!("/proc/self/ns/{name}")).map_err(|_| WorkerError::Preparation)
            })
            .collect::<Result<Vec<_>>>()?;
        policy::kernel(&text("/proc/sys/kernel/osrelease")?)?;
        Ok(Self {
            namespaces,
            executable,
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
        })
    }

    /// Membership was created before bwrap exec. Its --as-pid-1 contract leaves
    /// exactly the owned bwrap process and the native worker at this checkpoint.
    pub fn verify(&self, parent: i32, members: &[i32], cgroup_path: &Path) -> Result<()> {
        if members.len() != 2 || !members.contains(&parent) {
            return Err(WorkerError::Identity);
        }
        let worker = *members
            .iter()
            .find(|pid| **pid != parent)
            .ok_or(WorkerError::Identity)?;
        // pidfd pins this observed process instance while files are inspected.
        let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, worker, 0) };
        if descriptor < 0 {
            return Err(WorkerError::Identity);
        }
        let pidfd = unsafe { File::from_raw_fd(descriptor as i32) };
        let prefix = format!("/proc/{worker}");
        let before = text(&format!("{prefix}/stat"))?;
        policy::worker_status(
            &text(&format!("{prefix}/status"))?,
            worker,
            parent,
            self.uid,
            self.gid,
        )?;
        for (name, baseline) in NAMESPACES.iter().zip(&self.namespaces) {
            let actual =
                File::open(format!("{prefix}/ns/{name}")).map_err(|_| WorkerError::Identity)?;
            if same(&actual, baseline)? {
                return Err(WorkerError::Identity);
            }
        }
        let executable = File::open(format!("{prefix}/exe")).map_err(|_| WorkerError::Identity)?;
        if !same(&executable, &self.executable)? {
            return Err(WorkerError::Identity);
        }
        let expected = cgroup_path
            .strip_prefix("/sys/fs/cgroup")
            .map_err(|_| WorkerError::Identity)?;
        let expected = format!(
            "0::/{}\n",
            expected
                .to_str()
                .ok_or(WorkerError::Identity)?
                .trim_start_matches('/')
        );
        for pid in members {
            if text(&format!("/proc/{pid}/cgroup"))? != expected {
                return Err(WorkerError::Identity);
            }
        }
        mounts(&text(&format!("{prefix}/mountinfo"))?)?;
        for forbidden in ["/proc", "/sys", "/etc/passwd", "/run", "/home"] {
            match std::fs::symlink_metadata(format!("{prefix}/root{forbidden}")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(WorkerError::Identity),
            }
        }
        // The selected child cannot change identity during this observation.
        if process_start(&before)? != process_start(&text(&format!("{prefix}/stat"))?)? {
            return Err(WorkerError::Identity);
        }
        let mut event = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut event, 1, 0) } != 0 {
            return Err(WorkerError::Identity);
        }
        Ok(())
    }
}

fn same(first: &File, second: &File) -> Result<bool> {
    let first = first.metadata().map_err(|_| WorkerError::Identity)?;
    let second = second.metadata().map_err(|_| WorkerError::Identity)?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}
fn text(path: &str) -> Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| WorkerError::Identity)?;
    let mut bytes = Vec::new();
    file.take(32769)
        .read_to_end(&mut bytes)
        .map_err(|_| WorkerError::Identity)?;
    if bytes.len() > 32768 {
        return Err(WorkerError::Identity);
    }
    String::from_utf8(bytes).map_err(|_| WorkerError::Identity)
}
fn process_start(stat: &str) -> Result<u64> {
    // comm can contain spaces and parentheses. Fields after the final ')' start
    // at state(3); starttime(22) is index19 in that suffix.
    let suffix = stat.rsplit_once(')').ok_or(WorkerError::Identity)?.1;
    policy::number(
        suffix
            .split_whitespace()
            .nth(19)
            .ok_or(WorkerError::Identity)?,
    )
}

pub(super) fn mounts(text: &str) -> Result<()> {
    if text.len() > 32768 {
        return Err(WorkerError::Identity);
    }
    let roots = ["/", "/app/package", "/app/data", "/app/tmp", "/runtime"];
    let mut seen = [false; 5];
    for line in text.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 10 || !fields.contains(&"-") {
            return Err(WorkerError::Identity);
        }
        let Some(index) = roots.iter().position(|root| *root == fields[4]) else {
            continue;
        };
        if seen[index] {
            return Err(WorkerError::Identity);
        }
        seen[index] = true;
        let options = fields[5].split(',').collect::<Vec<_>>();
        if ([0, 1, 4].contains(&index) && !options.contains(&"ro"))
            || (index != 0 && (!options.contains(&"nosuid") || !options.contains(&"nodev")))
            || ([1, 2, 3].contains(&index) && !options.contains(&"noexec"))
        {
            return Err(WorkerError::Identity);
        }
    }
    if !seen.iter().all(|value| *value) {
        return Err(WorkerError::Identity);
    }
    Ok(())
}
