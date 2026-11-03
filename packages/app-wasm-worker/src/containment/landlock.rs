//! An installed App's WASI preopens are its real `data` and `tmp` directories.
//! Seccomp cannot tell paths apart, so Landlock confines every file access to
//! those two hierarchies before seccomp admits the path syscalls they need.
use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::PathBuf};

const EXECUTE: u64 = 1 << 0;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_SOCK: u64 = 1 << 9;
const MAKE_FIFO: u64 = 1 << 10;
const MAKE_BLOCK: u64 = 1 << 11;
/// ABI 1 rights: execute through make-symlink.
const ABI1: u64 = (1 << 13) - 1;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
const IOCTL_DEV: u64 = 1 << 15;
const RULE_PATH_BENEATH: libc::c_int = 1;
const CREATE_RULESET_VERSION: libc::c_uint = 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}
#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

fn unavailable(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::other(format!(
        "APP_SANDBOX_LANDLOCK: {operation}: {error}; App files need Linux Landlock (5.13 or later)"
    ))
}

pub(super) fn restrict(files: &[PathBuf]) -> io::Result<()> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    if abi < 1 {
        return Err(unavailable("landlock_create_ruleset(version)"));
    }
    let mut handled = ABI1;
    if abi >= 2 {
        handled |= REFER;
    }
    if abi >= 3 {
        handled |= TRUNCATE;
    }
    if abi >= 5 {
        handled |= IOCTL_DEV;
    }
    let attr = RulesetAttr {
        handled_access_fs: handled,
    };
    let ruleset = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    } as libc::c_int;
    if ruleset < 0 {
        return Err(unavailable("landlock_create_ruleset"));
    }
    let result = (|| {
        // Files only: no execution, devices, sockets or FIFOs inside them.
        let allowed =
            handled & !(EXECUTE | MAKE_CHAR | MAKE_SOCK | MAKE_FIFO | MAKE_BLOCK | IOCTL_DEV);
        for directory in files {
            let path = CString::new(directory.as_os_str().as_bytes())
                .map_err(|_| io::Error::other("APP_SANDBOX_LANDLOCK: invalid directory"))?;
            let fd = unsafe {
                libc::open(
                    path.as_ptr(),
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let rule = PathBeneath {
                allowed_access: allowed,
                parent_fd: fd,
            };
            let added = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset,
                    RULE_PATH_BENEATH,
                    &rule,
                    0u32,
                )
            };
            unsafe {
                libc::close(fd);
            }
            if added != 0 {
                return Err(unavailable("landlock_add_rule"));
            }
        }
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) } != 0 {
            return Err(unavailable("landlock_restrict_self"));
        }
        Ok(())
    })();
    unsafe {
        libc::close(ruleset);
    }
    result
}
