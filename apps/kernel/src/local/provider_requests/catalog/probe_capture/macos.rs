//! Match libproc pipe peer handles, which remain identifiable after reparenting.
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::fd::RawFd;

// Darwin's public proc_info.h definitions not provided by libc.
// https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h
#[repr(C)]
struct FileInfo {
    flags: u32,
    status: u32,
    offset: i64,
    kind: i32,
    guards: u32,
}
#[repr(C)]
struct PipeInfo {
    stat: libc::vinfo_stat,
    handle: u64,
    peer: u64,
    status: i32,
    reserved: i32,
}
#[repr(C)]
struct PipeFdInfo {
    file: FileInfo,
    pipe: PipeInfo,
}

fn pipe_info(pid: i32, fd: RawFd) -> Option<PipeFdInfo> {
    let mut info = MaybeUninit::<PipeFdInfo>::uninit();
    let size = size_of::<PipeFdInfo>() as i32;
    let result = unsafe { libc::proc_pidfdinfo(pid, fd, 6, info.as_mut_ptr().cast(), size) }; // PROC_PIDFDPIPEINFO
    (result == size).then(|| unsafe { info.assume_init() })
}

pub(super) fn terminate(fds: &[RawFd]) -> io::Result<()> {
    let own_pid = std::process::id() as i32;
    let peers = fds
        .iter()
        .map(|fd| pipe_info(own_pid, *fd).map(|info| info.pipe.peer))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(io::Error::last_os_error)?;
    // Bound enumeration memory even when another process has many descriptors.
    let mut pids = vec![0_i32; 65536];
    let mut descriptors = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        65536
    ];
    for _ in 0..3 {
        let count = unsafe {
            libc::proc_listallpids(
                pids.as_mut_ptr().cast(),
                (pids.len() * size_of::<i32>()) as i32,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        for pid in pids
            .iter()
            .take((count as usize).min(pids.len()))
            .copied()
            .filter(|pid| *pid > 0 && *pid != own_pid)
        {
            let bytes = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDLISTFDS,
                    0,
                    descriptors.as_mut_ptr().cast(),
                    (descriptors.len() * size_of::<libc::proc_fdinfo>()) as i32,
                )
            };
            if bytes <= 0 {
                continue;
            }
            for descriptor in descriptors
                .iter()
                .take((bytes as usize / size_of::<libc::proc_fdinfo>()).min(descriptors.len()))
            {
                if descriptor.proc_fdtype != libc::PROX_FDTYPE_PIPE as u32 {
                    continue;
                }
                if pipe_info(pid, descriptor.proc_fd)
                    .is_some_and(|info| info.pipe.handle != 0 && peers.contains(&info.pipe.handle))
                {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}
