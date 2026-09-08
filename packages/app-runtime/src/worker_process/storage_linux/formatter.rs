//! A fixed, descriptor-only ext4 formatter. The helper re-execs its own private
//! child mode so limits apply before mke2fs can write. No shell or install script.
use super::{files, model, Error, Result, DATA_BYTES, TMP_BYTES};
use crate::worker_process::spawn;
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io,
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::Path,
    time::{Duration, Instant},
};

const FORMATTER: &str = "/usr/sbin/mke2fs";

pub(super) fn format(
    image: &File,
    helper_lease: &File,
    capacity: u64,
    uuid: &str,
    uid: u32,
    gid: u32,
) -> Result<()> {
    validate(capacity, uuid, uid, gid)?;
    files::root_owned(image, false)?;
    if image.metadata()?.len() != capacity {
        return Err(Error::Identity);
    }
    let executable = trusted_executable(FORMATTER)?;
    // Pin the running root-owned binary, even if an installer atomically
    // selects another release while this helper still owns active leases.
    let helper = File::open("/proc/self/exe")?;
    files::root_owned(&helper, false)?;
    if helper.metadata()?.mode() & 0o111 == 0 {
        return Err(Error::Identity);
    }
    let null = File::options().read(true).write(true).open("/dev/null")?;
    let arguments = [
        "--format-child".to_string(),
        unsafe { libc::getpid() }.to_string(),
        capacity.to_string(),
        uuid.to_string(),
        uid.to_string(),
        gid.to_string(),
    ]
    .into_iter()
    .map(|s| CString::new(s).unwrap())
    .collect::<Vec<_>>();
    let pid = spawn::launch(
        &CString::new("/proc/self/fd/6").unwrap(),
        &arguments,
        &[
            &null,
            &null,
            &null,
            image,
            &executable,
            helper_lease,
            &helper,
        ],
    )
    .map_err(|_| Error::Io)?;
    let mut child = Child {
        pid,
        reaped: false,
        lost: false,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.exited()? {
            child.reap()?;
            break;
        }
        if Instant::now() >= deadline {
            return Err(Error::RecoveryRequired);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    image.sync_all()?;
    if image.metadata()?.len() != capacity {
        return Err(Error::Identity);
    }
    Ok(())
}

/// Root-only private mode, never a socket operation. FD3 is the preallocated
/// image; FD4 is the held root-owned formatter. FD5 retains the helper lock
/// while the formatter writes, including after a parent crash. execveat closes FD4.
pub(super) fn child(arguments: &[String]) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 || arguments.len() != 5 {
        return Err(Error::Invalid);
    }
    let parent = decimal::<i32>(&arguments[0])?;
    let capacity = decimal::<u64>(&arguments[1])?;
    let uid = decimal::<u32>(&arguments[3])?;
    let gid = decimal::<u32>(&arguments[4])?;
    let uuid = &arguments[2];
    validate(capacity, uuid, uid, gid)?;
    if parent <= 1 || unsafe { libc::getppid() } != parent {
        return Err(Error::Identity);
    }
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0
        || unsafe { libc::getppid() } != parent
    {
        return Err(Error::Identity);
    }
    let image = unsafe { File::from_raw_fd(3) };
    let executable = unsafe { File::from_raw_fd(4) };
    let helper_lease = unsafe { File::from_raw_fd(5) };
    let helper = unsafe { File::from_raw_fd(6) };
    files::root_owned(&helper, false)?;
    if helper.metadata()?.mode() & 0o111 == 0
        || files::identity(&helper)? != files::identity(&File::open("/proc/self/exe")?)?
    {
        return Err(Error::Identity);
    }
    files::root_owned(&helper_lease, true)?;
    files::root_owned(&image, false)?;
    files::root_owned(&executable, false)?;
    if image.metadata()?.len() != capacity || executable.metadata()?.mode() & 0o111 == 0 {
        return Err(Error::Identity);
    }
    for (resource, value) in [
        (libc::RLIMIT_FSIZE, capacity),
        (libc::RLIMIT_CPU, 30),
        (libc::RLIMIT_NOFILE, 32),
        (libc::RLIMIT_CORE, 0),
    ] {
        let limit = libc::rlimit {
            rlim_cur: value,
            rlim_max: value,
        };
        if unsafe { libc::setrlimit(resource, &limit) } != 0 {
            return Err(Error::Io);
        }
    }
    if unsafe { libc::fcntl(4, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
        || unsafe { libc::fcntl(6, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
    {
        return Err(Error::Io);
    }
    let arguments = command(capacity, uuid, uid, gid)
        .into_iter()
        .map(|value| CString::new(value).unwrap())
        .collect::<Vec<_>>();
    let mut argv = std::iter::once(c"mke2fs".as_ptr())
        .chain(arguments.iter().map(|v| v.as_ptr()))
        .collect::<Vec<_>>();
    argv.push(std::ptr::null());
    // Ignore host mke2fs feature defaults, and every caller environment variable.
    let env = [
        c"LANG=C".as_ptr(),
        c"TZ=UTC".as_ptr(),
        c"MKE2FS_CONFIG=/dev/null".as_ptr(),
        std::ptr::null(),
    ];
    unsafe {
        libc::execveat(
            4,
            c"".as_ptr(),
            argv.as_ptr(),
            env.as_ptr(),
            libc::AT_EMPTY_PATH,
        );
    }
    Err(Error::Io)
}
fn command(capacity: u64, uuid: &str, uid: u32, gid: u32) -> Vec<String> {
    [
        "-t".into(),
        "ext4".into(),
        "-b".into(),
        "4096".into(),
        "-I".into(),
        "256".into(),
        "-m".into(),
        "0".into(),
        "-O".into(),
        "none,has_journal,extent,filetype,sparse_super,large_file".into(),
        "-E".into(),
        format!("nodiscard,lazy_itable_init=0,lazy_journal_init=0,root_owner={uid}:{gid}"),
        "-U".into(),
        uuid.into(),
        "-F".into(),
        "-q".into(),
        "/proc/self/fd/3".into(),
        (capacity / 4096).to_string(),
    ]
    .into()
}
fn validate(capacity: u64, uuid: &str, uid: u32, gid: u32) -> Result<()> {
    model::uuid(uuid)?;
    if ![DATA_BYTES, TMP_BYTES].contains(&capacity)
        || [uid, gid].iter().any(|v| *v == 0 || *v == u32::MAX)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn decimal<T: std::str::FromStr>(value: &str) -> Result<T> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(Error::Invalid);
    }
    value.parse().map_err(|_| Error::Invalid)
}
fn trusted_executable(path: &str) -> Result<File> {
    let path = Path::new(path);
    let directory = files::root_directory(path.parent().ok_or(Error::Identity)?)?;
    let file = directory.read_file(path.file_name().ok_or(Error::Identity)?, false)?;
    files::root_owned(&file, false)?;
    if file.metadata()?.mode() & 0o111 == 0 {
        return Err(Error::Identity);
    }
    Ok(file)
}
struct Child {
    pid: i32,
    reaped: bool,
    lost: bool,
}
impl Child {
    fn exited(&mut self) -> Result<bool> {
        let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
        if unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as _,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } == 0
        {
            return Ok(unsafe { info.assume_init().si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        self.lost = error.raw_os_error() == Some(libc::ECHILD);
        Err(Error::Io)
    }
    fn reap(&mut self) -> Result<()> {
        if !self.lost {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
        let mut status = 0;
        loop {
            if unsafe { libc::waitpid(self.pid, &mut status, 0) } == self.pid {
                self.reaped = true;
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                self.reaped = true;
                return Err(Error::Io);
            }
        }
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
            Ok(())
        } else {
            Err(Error::Io)
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.reap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formatter_is_fixed_to_held_image_and_explicit_block_units() {
        let uuid = "11111111-1111-4111-8111-111111111111";
        for capacity in [DATA_BYTES, TMP_BYTES] {
            let args = command(capacity, uuid, 1000, 1000);
            assert_eq!(args[args.len() - 2], "/proc/self/fd/3");
            assert_eq!(
                args.last().unwrap().parse::<u64>().unwrap() * 4096,
                capacity
            );
            assert!(args.contains(
                &"nodiscard,lazy_itable_init=0,lazy_journal_init=0,root_owner=1000:1000".to_owned()
            ));
        }
        assert!(validate(1, uuid, 1000, 1000).is_err());
        assert!(validate(DATA_BYTES, uuid, 0, 1000).is_err());
        assert!(decimal::<u64>("01").is_err());
    }
}
