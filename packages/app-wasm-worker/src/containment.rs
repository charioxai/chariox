//! Native worker boundary, installed before the ready handshake or any threads.
//! Files, network and subprocesses belong to kernel brokers, never native WASI.
use std::io;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

/// Hard process ceilings, independent of guest linear-memory/epoch limits.
pub const ADDRESS_SPACE_BYTES: u64 = 1024 * 1024 * 1024;
pub const CPU_SECONDS: u64 = 300;
pub const FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const DESCRIPTORS: u64 = 64;

/// Call once, on the initial native thread, after reading supervisor inputs.
/// Failure is fatal: there is no uncontained execution mode.
pub fn enter() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    linux::enter()?;
    #[cfg(target_os = "macos")]
    macos::enter()?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "worker containment requires Linux or macOS",
    ));
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ceilings() -> io::Result<()> {
    for (resource, value) in [
        (libc::RLIMIT_AS, ADDRESS_SPACE_BYTES),
        (libc::RLIMIT_CPU, CPU_SECONDS),
        (libc::RLIMIT_FSIZE, FILE_BYTES),
        (libc::RLIMIT_NOFILE, DESCRIPTORS),
        (libc::RLIMIT_CORE, 0),
    ] {
        let mut old = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(resource, &mut old) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let hard = (value + u64::from(resource == libc::RLIMIT_CPU)) as libc::rlim_t;
        let value = (value as libc::rlim_t).min(old.rlim_cur).min(old.rlim_max);
        let limit = libc::rlimit {
            rlim_cur: value,
            rlim_max: hard.min(old.rlim_max),
        };
        if unsafe { libc::setrlimit(resource, &limit) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
