//! MP-08 / MP-10 / MP-11: record parent lineage and verify process birth identity before signals.
//! proc_listchildpids returns a PID count: https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c.
use std::collections::BTreeSet;
use std::io;
use std::mem::{size_of, MaybeUninit};
pub(super) struct Descendant {
    pid: u32,
    birth: (u64, u64),
}
fn info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    let result = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (result == size).then(|| unsafe { info.assume_init() })
}
impl Descendant {
    pub(super) fn alive(&self) -> bool {
        info(self.pid).is_some_and(|i| (i.pbi_start_tvsec, i.pbi_start_tvusec) == self.birth)
    }
    pub(super) fn signal(&self, signal: i32) -> io::Result<()> {
        let pid = super::signal_pid(self.pid.into())?;
        if self.alive() && unsafe { libc::kill(pid, signal) } < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(e);
            }
        }
        Ok(())
    }
}
pub(super) fn observe(
    root: Option<u32>,
    owned: &mut Vec<Descendant>,
    include_group: bool,
) -> io::Result<()> {
    let mut parents = root
        .into_iter()
        .chain(owned.iter().filter(|p| p.alive()).map(|p| p.pid))
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(pid) = parents.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let mut children = [0i32; 1024];
        let count = unsafe {
            libc::proc_listchildpids(
                pid as i32,
                children.as_mut_ptr().cast(),
                (children.len() * size_of::<i32>()) as i32,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count as usize >= children.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "probe descendant limit",
            ));
        }
        let mut candidates = children
            .iter()
            .take(count as usize)
            .copied()
            .collect::<Vec<_>>();
        if include_group && root == Some(pid) {
            let mut members = [0i32; 1024];
            let bytes = unsafe {
                libc::proc_listpids(
                    libc::PROC_PGRP_ONLY,
                    pid,
                    members.as_mut_ptr().cast(),
                    (members.len() * size_of::<i32>()) as i32,
                )
            };
            if bytes < 0 {
                return Err(io::Error::last_os_error());
            }
            if bytes as usize >= members.len() * size_of::<i32>() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "probe descendant limit",
                ));
            }
            candidates.extend(
                members
                    .iter()
                    .take(bytes as usize / size_of::<i32>())
                    .copied(),
            );
        }
        for child in candidates {
            if super::signal_pid(child.into()).is_err() {
                continue;
            }
            let child = child as u32;
            if owned.iter().any(|p| p.pid == child && p.alive()) {
                continue;
            }
            let Some(i) = info(child) else {
                continue;
            };
            let parent_owned = root == Some(pid) || owned.iter().any(|p| p.pid == pid && p.alive());
            if parent_owned && (i.pbi_ppid == pid || (root == Some(pid) && i.pbi_pgid == pid)) {
                if owned.len() >= 1024 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "probe descendant limit",
                    ));
                }
                owned.push(Descendant {
                    pid: child,
                    birth: (i.pbi_start_tvsec, i.pbi_start_tvusec),
                });
                parents.push(child);
            }
        }
    }
    Ok(())
}
