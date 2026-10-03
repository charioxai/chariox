//! Fixed Apple tools, using the same clean-environment/session spawn primitive.
use super::{Error, Result};
use crate::worker_process::spawn;
use std::{
    ffi::CString,
    fs::File,
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct Context<'a> {
    pub lease: &'a File,
    pub deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Hdiutil,
    Diskutil,
    Plutil,
}
impl Tool {
    fn path(self) -> &'static str {
        match self {
            Self::Hdiutil => "/usr/bin/hdiutil",
            Self::Diskutil => "/usr/sbin/diskutil",
            Self::Plutil => "/usr/bin/plutil",
        }
    }
}

pub(super) fn plist(
    tool: Tool,
    arguments: &[String],
    context: Context<'_>,
) -> Result<serde_json::Value> {
    let xml = run(tool, arguments, &[], context)?;
    let json = run(
        Tool::Plutil,
        &strings(&["-convert", "json", "-o", "-", "-"]),
        &xml,
        context,
    )?;
    serde_json::from_slice(&json).map_err(|_| Error::Metadata)
}

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}

struct Child {
    pid: libc::pid_t,
    reaped: bool,
    lost: bool,
}
impl Child {
    fn exited(&mut self) -> Result<bool> {
        let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(unsafe { info.assume_init().si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        self.lost = error.raw_os_error() == Some(libc::ECHILD);
        Err(Error::Command)
    }
    fn reap(&mut self) -> Result<()> {
        if !self.lost {
            // Keep the leader unreaped until group termination to prevent PID reuse.
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(self.pid, &mut status, 0) };
            if result == self.pid {
                self.reaped = true;
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                self.reaped = true;
                return Err(Error::Command);
            }
        }
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
            Ok(())
        } else {
            Err(Error::Command)
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

/// No shell, inherited credentials, arbitrary executable, detached waiter, or
/// unbounded stdout/stderr. DiskImages services may own an attached image after
/// hdiutil exits; image identity and durable recovery own that separate lifetime.
pub(super) fn run(
    tool: Tool,
    arguments: &[String],
    input: &[u8],
    context: Context<'_>,
) -> Result<Vec<u8>> {
    if arguments.len() > 32
        || arguments.iter().any(|v| v.len() > 2048 || v.contains('\0'))
        || input.len() > MAX_OUTPUT
    {
        return Err(Error::Metadata);
    }
    if Instant::now() >= context.deadline {
        return Err(Error::CommandTimeout);
    }
    let program = CString::new(tool.path()).unwrap();
    let arguments = arguments
        .iter()
        .map(|v| CString::new(v.as_str()).map_err(|_| Error::Metadata))
        .collect::<Result<Vec<_>>>()?;
    let (mut writer, stdin) = UnixStream::pair()?;
    let (mut output, stdout) = UnixStream::pair()?;
    let (mut diagnostic, stderr) = UnixStream::pair()?;
    for stream in [&writer, &output, &diagnostic] {
        stream.set_nonblocking(true)?;
    }
    let stdin = File::from(std::os::fd::OwnedFd::from(stdin));
    let stdout = File::from(std::os::fd::OwnedFd::from(stdout));
    let stderr = File::from(std::os::fd::OwnedFd::from(stderr));
    let null = File::open("/dev/null")?;
    let pid = spawn::launch(
        &program,
        &arguments,
        &[&stdin, &stdout, &stderr, context.lease, &null],
    )
    .map_err(|_| Error::Command)?;
    let mut child = Child {
        pid,
        reaped: false,
        lost: false,
    };
    drop((stdin, stdout, stderr));
    let deadline = context
        .deadline
        .min(Instant::now() + Duration::from_secs(if tool == Tool::Plutil { 5 } else { 90 }));
    let mut bytes = Vec::new();
    let mut diagnostics = 0;
    let mut written = 0;
    let mut input_closed = false;
    loop {
        if Instant::now() >= deadline {
            return Err(Error::CommandTimeout);
        }
        if written < input.len() {
            match writer.write(&input[written..]) {
                Ok(0) => return Err(Error::Command),
                Ok(count) => written += count,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => return Err(Error::Command),
            }
        }
        if written == input.len() && !input_closed {
            writer.shutdown(std::net::Shutdown::Write)?;
            input_closed = true;
        }
        drain(&mut output, &mut bytes, MAX_OUTPUT)?;
        let mut chunk = Vec::new();
        drain(&mut diagnostic, &mut chunk, 65536)?;
        diagnostics += chunk.len();
        if diagnostics > 65536 {
            return Err(Error::CommandOutput);
        }
        if child.exited()? {
            // At most one bounded final read; a service retaining the pipe cannot
            // keep this request alive. Truncated plist fails before any mount action.
            for _ in 0..8 {
                drain(&mut output, &mut bytes, MAX_OUTPUT)?;
            }
            child.reap()?;
            return Ok(bytes);
        }
        let mut poll = [
            libc::pollfd {
                fd: output.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: diagnostic.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let result = unsafe { libc::poll(poll.as_mut_ptr(), 2, 20) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(Error::Command);
        }
    }
}

fn drain(stream: &mut UnixStream, output: &mut Vec<u8>, limit: usize) -> Result<()> {
    let mut buffer = [0; 8192];
    for _ in 0..16 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if output.len() + count > limit {
                    return Err(Error::CommandOutput);
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Error::Command),
        }
    }
    Ok(())
}
