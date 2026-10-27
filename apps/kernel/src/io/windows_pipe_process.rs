//! Windows pipe producers start suspended so no descendant can escape job assignment.
use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

// These kernel32 APIs need no additional crate features. Keep their ABI here
// with the owned handles they serve.
type Handle = *mut c_void;
#[repr(C)]
struct ThreadEntry {
    size: u32,
    usage: u32,
    id: u32,
    owner: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}
#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn PeekNamedPipe(
        pipe: Handle,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        remaining: *mut u32,
    ) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> Handle;
    fn ResumeThread(thread: Handle) -> u32;
}

struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(crate) trait Pipe: AsRawHandle {}
impl<T: AsRawHandle> Pipe for T {}

pub(crate) fn available(pipe: &impl Pipe) -> io::Result<usize> {
    readiness(pipe).map(|ready| ready.unwrap_or(0))
}

// None is genuine EOF; Some(0) is an open pipe with no bytes yet.
pub(crate) fn readiness(pipe: &impl Pipe) -> io::Result<Option<usize>> {
    let mut count = 0;
    let result = unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut count,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(109) {
            return Ok(None);
        } // ERROR_BROKEN_PIPE
        return Err(error);
    }
    Ok(Some(count as usize))
}

pub(crate) struct Process {
    pub(crate) child: Child,
    job: OwnedHandle,
    stopped: bool,
}

impl Process {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = OwnedHandle(job);
        command.creation_flags(0x4); // CREATE_SUSPENDED
        let mut process = Self {
            child: command.spawn()?,
            job,
            stopped: false,
        };
        if unsafe { AssignProcessToJobObject(process.job.0, process.child.as_raw_handle()) } == 0 {
            let error = io::Error::last_os_error();
            process.stop()?;
            return Err(error);
        }
        resume(process.child.id())?;
        Ok(process)
    }

    pub(crate) fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        let cleanup = if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        let _ = self.child.kill();
        let reaped = self.child.wait().map(|_| ());
        self.stopped = true;
        cleanup.and(reaped)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn resume(pid: u32) -> io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(0x4, 0) }; // TH32CS_SNAPTHREAD
    if snapshot == -1_isize as Handle {
        return Err(io::Error::last_os_error());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: ThreadEntry = unsafe { std::mem::zeroed() };
    entry.size = std::mem::size_of::<ThreadEntry>() as u32;
    let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
    while found != 0 {
        if entry.owner == pid {
            let thread = unsafe { OpenThread(0x2, 0, entry.id) }; // THREAD_SUSPEND_RESUME
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            let thread = OwnedHandle(thread);
            if unsafe { ResumeThread(thread.0) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        found = unsafe { Thread32Next(snapshot.0, &mut entry) };
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "producer primary thread was not found",
    ))
}
