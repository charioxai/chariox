//! Port of the v1 native worker's seccomp allowlist and pthread checks
//! (apps/app-worker/src/sandbox_linux.c), narrowed to broker-only native I/O.
//! Path syscalls are absent from the allowlist, including on older kernels.
use std::io;

const ALLOW: u32 = 0x7fff0000;
const DENY: u32 = 0x00050000 | libc::EPERM as u32;
const KILL: u32 = 0x80000000;
#[cfg(target_arch = "x86_64")]
const ARCH: u32 = 0xc000003e;
#[cfg(target_arch = "aarch64")]
const ARCH: u32 = 0xc00000b7;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("App worker containment supports Linux x86_64 and aarch64");

fn ins(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}
fn load(offset: u32) -> libc::sock_filter {
    ins(0x20, offset, 0, 0)
}
fn eq(k: u32, yes: u8, no: u8) -> libc::sock_filter {
    ins(0x15, k, yes, no)
}
fn ret(value: u32) -> libc::sock_filter {
    ins(0x06, value, 0, 0)
}
fn arg(index: u32) -> u32 {
    16 + index * 8
}
fn case(filter: &mut Vec<libc::sock_filter>, syscall: libc::c_long, body: &[libc::sock_filter]) {
    filter.push(eq(
        syscall as u32,
        0,
        body.len().try_into().expect("bounded seccomp block"),
    ));
    filter.extend_from_slice(body);
    filter.push(load(0));
}

pub(super) fn enter() -> io::Result<()> {
    // Descriptor cleanup must precede threads sharing the descriptor table.
    // The epoch ticker and Tokio threads must all be born after this point.
    if std::fs::read_dir("/proc/self/task")?.count() != 1 {
        return Err(io::Error::other(
            "APP_SANDBOX_ORDER: containment must precede threads",
        ));
    }
    // READ_IMPLIES_EXEC would make a nominal RW mmap executable despite the
    // prot argument checked below. Never inherit that legacy personality.
    let personality = unsafe { libc::personality(0xffffffff) };
    if personality < 0 || personality & libc::READ_IMPLIES_EXEC != 0 {
        return Err(io::Error::other(
            "APP_SANDBOX_PERSONALITY: implicit executable mappings refused",
        ));
    }
    if unsafe { libc::syscall(libc::SYS_close_range, 5u32, u32::MAX, 0u32) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0
        || unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    super::ceilings()?;
    let pid = unsafe { libc::getpid() };
    if pid <= 1 {
        return Err(io::Error::other("APP_SANDBOX_PID: invalid worker PID"));
    }
    let mut filter = vec![load(4), eq(ARCH, 1, 0), ret(KILL), load(0)];
    case(
        &mut filter,
        libc::SYS_clone3,
        &[ret(0x00050000 | libc::ENOSYS as u32)],
    );
    let required = (libc::CLONE_VM
        | libc::CLONE_FS
        | libc::CLONE_FILES
        | libc::CLONE_SIGHAND
        | libc::CLONE_THREAD
        | libc::CLONE_SYSVSEM) as u32;
    let allowed = required
        | (libc::CLONE_SETTLS
            | libc::CLONE_PARENT_SETTID
            | libc::CLONE_CHILD_SETTID
            | libc::CLONE_CHILD_CLEARTID) as u32;
    case(
        &mut filter,
        libc::SYS_clone,
        &[
            load(arg(0) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(0)),
            ins(0x54, !allowed, 0, 0),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(0)),
            ins(0x54, required, 0, 0),
            eq(required, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    case(
        &mut filter,
        libc::SYS_prlimit64,
        &[
            load(arg(0) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(0)),
            eq(0, 2, 0),
            eq(pid as u32, 1, 0),
            ret(DENY),
            load(arg(2)),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(2) + 4),
            eq(0, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    case(
        &mut filter,
        libc::SYS_tgkill,
        &[
            load(arg(0) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(0)),
            eq(pid as u32, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    // Wasmtime publishes AArch64 code with private SYNC_CORE and its matching
    // registration. Global barriers, other commands and nonzero flags stay out.
    #[cfg(target_arch = "aarch64")]
    case(
        &mut filter,
        libc::SYS_membarrier,
        &[
            load(arg(0) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(0)),
            eq(32, 2, 0),
            eq(64, 1, 0),
            ret(DENY),
            load(arg(1) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(1)),
            eq(0, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    // Wasmtime publishes RW code as RX. Simultaneous writable/executable
    // mappings and pkey_mprotect are never admitted, including in new threads.
    for syscall in [libc::SYS_mmap, libc::SYS_mprotect] {
        case(
            &mut filter,
            syscall,
            &[
                load(arg(2)),
                ins(0x54, (libc::PROT_WRITE | libc::PROT_EXEC) as u32, 0, 0),
                eq((libc::PROT_WRITE | libc::PROT_EXEC) as u32, 0, 1),
                ret(DENY),
                ret(ALLOW),
            ],
        );
    }
    case(
        &mut filter,
        libc::SYS_ioctl,
        &[
            load(arg(1) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(1)),
            eq(libc::FIONBIO as u32, 2, 0),
            eq(libc::FIONREAD as u32, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    case(
        &mut filter,
        libc::SYS_setsockopt,
        &[
            load(arg(1) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(1)),
            eq(libc::SOL_SOCKET as u32, 1, 0),
            ret(DENY),
            load(arg(2) + 4),
            eq(0, 1, 0),
            ret(DENY),
            load(arg(2)),
            eq(libc::SO_RCVTIMEO as u32, 2, 0),
            eq(libc::SO_SNDTIMEO as u32, 1, 0),
            ret(DENY),
            ret(ALLOW),
        ],
    );
    for syscall in [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_close,
        libc::SYS_fcntl,
        libc::SYS_dup,
        libc::SYS_dup3,
        libc::SYS_lseek,
        libc::SYS_fstat,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_ftruncate,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_rt_sigtimedwait,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_gettid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_gettimeofday,
        libc::SYS_nanosleep,
        libc::SYS_clock_nanosleep,
        libc::SYS_futex,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_getrusage,
        libc::SYS_getrlimit,
        libc::SYS_eventfd2,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_ppoll,
        libc::SYS_pselect6,
        libc::SYS_pipe2,
        libc::SYS_recvfrom,
        libc::SYS_sendto,
        libc::SYS_getsockopt,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_shutdown,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ] {
        case(&mut filter, syscall, &[ret(ALLOW)]);
    }
    #[cfg(target_arch = "x86_64")]
    for syscall in [
        libc::SYS_arch_prctl,
        libc::SYS_epoll_wait,
        libc::SYS_poll,
        libc::SYS_select,
        libc::SYS_dup2,
    ] {
        case(&mut filter, syscall, &[ret(ALLOW)]);
    }
    filter.push(ret(DENY));
    let program = libc::sock_fprog {
        len: filter.len().try_into().expect("bounded seccomp program"),
        filter: filter.as_mut_ptr(),
    };
    // Apply the policy to every thread, including any created by system APIs.
    if unsafe { libc::syscall(libc::SYS_seccomp, 1, 1, &program) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
