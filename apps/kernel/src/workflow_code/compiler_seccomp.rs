//! MP-11 F14: allow Node threads, deny process and namespace creation.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
use super::isolation_error;

fn statement(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}
fn branch(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

pub(super) fn compiler_filter() -> Result<Vec<libc::sock_filter>, crate::DaemonError> {
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc00000b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err(isolation_error(
        "compiler syscall policy is unavailable on this architecture",
    ));
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        const LD: u16 = 0x20;
        const JEQ: u16 = 0x15;
        const JSET: u16 = 0x45;
        const RET: u16 = 0x06;
        const ALLOW: u32 = 0x7fff0000;
        const ERRNO: u32 = 0x00050000;
        let mut rules = vec![
            statement(LD, 4),
            branch(JEQ, arch, 1, 0),
            statement(RET, 0x80000000),
            statement(LD, 0),
            // clone3 arguments are a pointer, so require libc's clone fallback.
            branch(JEQ, libc::SYS_clone3 as u32, 0, 1),
            statement(RET, ERRNO | libc::ENOSYS as u32),
        ];
        let denied = vec![libc::SYS_unshare, libc::SYS_setns];
        #[cfg(target_arch = "x86_64")]
        let denied = {
            let mut denied = denied;
            denied.extend([libc::SYS_fork, libc::SYS_vfork]);
            denied
        };
        for syscall in denied {
            rules.extend([
                branch(JEQ, syscall as u32, 0, 1),
                statement(RET, ERRNO | libc::EPERM as u32),
            ]);
        }
        // The same audit architecture also includes the x32 ABI.
        #[cfg(target_arch = "x86_64")]
        rules.extend([
            branch(JSET, 0x40000000, 0, 1),
            statement(RET, ERRNO | libc::EPERM as u32),
        ]);
        rules.extend([
            branch(JEQ, libc::SYS_clone as u32, 0, 3),
            statement(LD, 16),
            branch(JSET, libc::CLONE_THREAD as u32, 1, 0),
            statement(RET, ERRNO | libc::EPERM as u32),
            statement(RET, ALLOW),
        ]);
        Ok(rules)
    }
}

/// Called after stdio setup and close-on-exec marking in Command's child.
/// Only the filter descriptor survives into bwrap, which consumes and closes it.
pub(super) fn install_filter_fd(filter: &[libc::sock_filter]) -> std::io::Result<()> {
    unsafe {
        let fd = libc::syscall(
            libc::SYS_memfd_create,
            c"chariox-compiler-policy".as_ptr(),
            libc::MFD_CLOEXEC,
        ) as i32;
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let bytes =
            std::slice::from_raw_parts(filter.as_ptr().cast::<u8>(), std::mem::size_of_val(filter));
        let result = (|| {
            let mut offset = 0;
            while offset < bytes.len() {
                let written =
                    libc::write(fd, bytes[offset..].as_ptr().cast(), bytes.len() - offset);
                if written < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if written == 0 {
                    return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
                }
                offset += written as usize;
            }
            if libc::lseek(fd, 0, libc::SEEK_SET) < 0
                || libc::dup2(fd, 3) < 0
                || libc::fcntl(3, libc::F_SETFD, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })();
        if fd != 3 {
            libc::close(fd);
        }
        result
    }
}
