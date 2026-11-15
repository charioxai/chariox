//! The environment a running process holds, which is how a workload's detached
//! jobs are told from strangers: they inherited its marker.

use std::io;

/// The process's environment as NUL-separated `NAME=value` entries. Empty
/// while the process is mid-exec or a zombie; `PermissionDenied` for another
/// user's process.
#[cfg(target_os = "linux")]
pub(super) fn read(pid: u32) -> io::Result<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ"))
}

#[cfg(target_os = "macos")]
pub(super) fn read(pid: u32) -> io::Result<Vec<u8>> {
    let pid = libc::c_int::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PID out of range"))?;
    let mut request = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = 0usize;
    // SAFETY: a null buffer only asks for the size of the answer.
    let sized = unsafe {
        libc::sysctl(
            request.as_mut_ptr(),
            request.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if sized != 0 {
        return Err(unreadable());
    }
    // The process may grow its arguments between the two calls.
    let mut buffer = vec![0u8; size + size / 4 + 64];
    size = buffer.len();
    // SAFETY: `buffer` is writable for `size` bytes.
    let read = unsafe {
        libc::sysctl(
            request.as_mut_ptr(),
            request.len() as libc::c_uint,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return Err(unreadable());
    }
    buffer.truncate(size);
    environment_of(&buffer)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn read(_pid: u32) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native process environment requires Linux or macOS",
    ))
}

/// Another user's process answers EPERM, and a system process or one mid-exec
/// EINVAL: only the first never becomes readable.
#[cfg(target_os = "macos")]
fn unreadable() -> io::Error {
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EPERM) => io::Error::from(io::ErrorKind::PermissionDenied),
        _ => error,
    }
}

/// `KERN_PROCARGS2` lays out the argument count, the executable path, padding
/// NULs, the arguments and then the environment.
#[cfg(any(target_os = "macos", test))]
fn environment_of(raw: &[u8]) -> io::Result<Vec<u8>> {
    let malformed = || io::Error::other("invalid native process arguments");
    let count = raw
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i32::from_ne_bytes)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(malformed)?;
    let mut rest = &raw[4..];
    let path_end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(malformed)?;
    rest = &rest[path_end..];
    let padding = rest
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(rest.len());
    rest = &rest[padding..];
    for _ in 0..count {
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(malformed)?;
        rest = &rest[end + 1..];
    }
    let entries = rest.len() - rest.iter().rev().take_while(|byte| **byte == 0).count();
    Ok(rest[..entries].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(count: i32, path: &str, arguments: &[&str], environment: &[&str]) -> Vec<u8> {
        let mut raw = count.to_ne_bytes().to_vec();
        raw.extend(path.bytes());
        raw.extend([0, 0, 0]);
        for text in arguments.iter().chain(environment) {
            raw.extend(text.bytes());
            raw.push(0);
        }
        raw
    }

    #[test]
    fn the_environment_follows_the_arguments() {
        let raw = layout(2, "/bin/sleep", &["sleep", "5"], &["A=1", "MARK=x y"]);
        assert_eq!(environment_of(&raw).unwrap(), b"A=1\0MARK=x y");
    }

    #[test]
    fn a_process_without_environment_reads_empty() {
        let raw = layout(1, "/bin/true", &["true"], &[]);
        assert!(environment_of(&raw).unwrap().is_empty());
    }

    #[test]
    fn a_truncated_answer_is_refused() {
        let mut raw = layout(3, "/bin/sleep", &["sleep"], &[]);
        raw.truncate(raw.len() - 1);
        assert!(environment_of(&raw).is_err());
        assert!(environment_of(&[1, 0]).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reads_its_own_environment() {
        let environment = read(std::process::id()).unwrap();
        assert!(environment
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"PATH=")));
    }
}
