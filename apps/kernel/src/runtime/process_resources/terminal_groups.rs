//! Terminal ownership changes need current ancestry, independently of ps sampling.
use std::collections::BTreeSet;
use std::io;

struct ProcessGroup {
    pid: u32,
    parent: u32,
    group: u32,
}

pub(super) fn read(root: u32) -> io::Result<Vec<u32>> {
    let rows = rows()?;
    let own_group = rows
        .iter()
        .find(|row| row.pid == root)
        .map(|row| row.group)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "terminal shell has exited"))?;
    let mut below = BTreeSet::from([root]);
    loop {
        let before = below.len();
        for row in &rows {
            if below.contains(&row.parent) {
                below.insert(row.pid);
            }
        }
        if before == below.len() {
            break;
        }
    }
    Ok(rows
        .into_iter()
        .filter(|row| {
            row.pid != root && below.contains(&row.pid) && row.group > 1 && row.group != own_group
        })
        .map(|row| row.group)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

#[cfg(target_os = "linux")]
fn rows() -> io::Result<Vec<ProcessGroup>> {
    let mut rows = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .filter(|pid| *pid > 1)
        else {
            continue;
        };
        let bytes = match std::fs::read(entry.path().join("stat")) {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT | libc::ESRCH | libc::EACCES | libc::EPERM)
                ) =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        let text = String::from_utf8_lossy(&bytes);
        let (_, fields) = text
            .rsplit_once(") ")
            .ok_or_else(|| io::Error::other("invalid native process metadata"))?;
        let mut fields = fields.split_ascii_whitespace().skip(1);
        let [Some(parent), Some(group)] = [
            fields.next().and_then(|field| field.parse().ok()),
            fields.next().and_then(|field| field.parse().ok()),
        ] else {
            return Err(io::Error::other("invalid native process ancestry"));
        };
        rows.push(ProcessGroup { pid, parent, group });
    }
    Ok(rows)
}

#[cfg(target_os = "macos")]
fn rows() -> io::Result<Vec<ProcessGroup>> {
    let mut pids = vec![0_i32; 65536];
    let count = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count as usize >= pids.len() {
        return Err(io::Error::other(
            "native process snapshot exceeded its capacity",
        ));
    }
    let mut rows = Vec::new();
    for pid in pids.into_iter().take(count as usize).filter(|pid| *pid > 1) {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if read != size {
            let error = io::Error::last_os_error();
            if read == 0
                && matches!(
                    error.raw_os_error(),
                    Some(libc::ESRCH | libc::EPERM | libc::EACCES)
                )
            {
                continue;
            }
            return Err(io::Error::other("incomplete native process metadata"));
        }
        rows.push(ProcessGroup {
            pid: info.pbi_pid,
            parent: info.pbi_ppid,
            group: info.pbi_pgid,
        });
    }
    Ok(rows)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rows() -> io::Result<Vec<ProcessGroup>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native terminal process ancestry requires Linux or macOS",
    ))
}
