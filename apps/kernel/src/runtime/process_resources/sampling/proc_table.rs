//! The process table as Linux keeps it in /proc: one stat file per process, read
//! on the sampler's own thread. Starting `ps` five times a second cost the kernel
//! 11-14 s of CPU a minute with 12 watched commands on a host of 2800
//! processes; reading every stat file at every tick still cost about half of
//! that, so the sampler reads a process's stat file once ([`Table`]).
use super::{shortage, Row, SamplingError};
use std::collections::HashMap;

/// Every process, each read now.
pub(super) fn read() -> Result<Vec<Row>, SamplingError> {
    Table::default().read()
}

/// The processes the sampler read before, by the inode of their /proc entry.
/// A process keeps its inode while it lives and one that takes over its PID
/// gets a new one, so a process listed with the same inode, under the same
/// parent, is not read again: its row keeps the parent, birth and command, and
/// the resources, of its first read. Trees read their own processes now
/// ([`current`]).
#[derive(Default)]
pub(super) struct Table {
    known: HashMap<u32, Known>,
}

struct Known {
    inode: u64,
    /// The inode of the parent's entry at that read: the parent ended (and
    /// the process was given to another) once the parent's entry changed.
    parent_inode: Option<u64>,
    start: u64,
    row: Row,
}

impl Table {
    pub(super) fn read(&mut self) -> Result<Vec<Row>, SamplingError> {
        use std::os::unix::fs::DirEntryExt as _;
        let clock = Clock::now()?;
        let mut listed = HashMap::with_capacity(self.known.len());
        for entry in std::fs::read_dir("/proc").map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;
            if let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            {
                listed.insert(pid, entry.ino());
            }
        }
        let mut known = HashMap::with_capacity(listed.len());
        let mut rows = Vec::with_capacity(listed.len());
        for (&pid, &inode) in &listed {
            let entry = match self.known.remove(&pid) {
                Some(mut entry)
                    if entry.inode == inode
                        && listed.get(&entry.row.parent).copied() == entry.parent_inode =>
                {
                    entry.row.age_ms = clock.age_ms(entry.start);
                    entry.row.sampled_at_ms = clock.sampled_at_ms;
                    entry
                }
                _ => {
                    let Some((row, start)) = clock.read(pid)? else {
                        continue;
                    };
                    Known {
                        inode,
                        parent_inode: listed.get(&row.parent).copied(),
                        start,
                        row,
                    }
                }
            };
            rows.push(entry.row.clone());
            known.insert(pid, entry);
        }
        self.known = known;
        if rows.is_empty() {
            return Err(SamplingError::Unusable(
                "process resource monitor read an empty process table from /proc".into(),
            ));
        }
        Ok(rows)
    }
}

/// The rows of `pids` as their stat files read now, without the processes
/// that ended.
pub(super) fn current(pids: impl IntoIterator<Item = u32>) -> Result<Vec<Row>, SamplingError> {
    let clock = Clock::now()?;
    let mut rows = Vec::new();
    for pid in pids {
        rows.extend(clock.read(pid)?.map(|(row, _)| row));
    }
    Ok(rows)
}

struct Clock {
    hertz: u64,
    page: u64,
    /// Clock ticks since boot.
    now: u64,
    sampled_at_ms: u64,
}

impl Clock {
    fn now() -> Result<Self, SamplingError> {
        let sampled_at_ms = crate::session::unix_epoch_ms();
        // SAFETY: sysconf only reads system configuration.
        let (hertz, page) = unsafe {
            (
                libc::sysconf(libc::_SC_CLK_TCK),
                libc::sysconf(libc::_SC_PAGESIZE),
            )
        };
        let (Ok(hertz @ 1..), Ok(page @ 1..)) = (u64::try_from(hertz), u64::try_from(page)) else {
            return Err(SamplingError::Unusable(
                "process resource monitor cannot read the clock tick or page size of this system"
                    .into(),
            ));
        };
        let now = std::fs::read_to_string("/proc/uptime")
            .map_err(unreadable)?
            .split_whitespace()
            .next()
            .and_then(|seconds| seconds.parse::<f64>().ok())
            .map(|seconds| (seconds * hertz as f64) as u64)
            .ok_or_else(|| {
                SamplingError::Unusable("process resource monitor cannot read /proc/uptime".into())
            })?;
        Ok(Self {
            hertz,
            page,
            now,
            sampled_at_ms,
        })
    }

    fn age_ms(&self, start: u64) -> u64 {
        self.now.saturating_sub(start).saturating_mul(1000) / self.hertz
    }

    /// The row of `pid` and its start in clock ticks; none once it ended.
    fn read(&self, pid: u32) -> Result<Option<(Row, u64)>, SamplingError> {
        let stat = match std::fs::read(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            // It ended after it was listed, or this kernel may not read it.
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT | libc::ESRCH | libc::EACCES | libc::EPERM)
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(unreadable(error)),
        };
        Ok(row(pid, &String::from_utf8_lossy(&stat), self))
    }
}

fn unreadable(error: std::io::Error) -> SamplingError {
    match shortage(&error) {
        Some(errno) => SamplingError::Short(errno),
        None => SamplingError::Unusable(format!(
            "process resource monitor cannot read /proc: {error}; the kernel needs procfs mounted"
        )),
    }
}

/// `pid (comm) state ppid ...`: the command may hold spaces and parentheses, so
/// the fields start after its last parenthesis.
fn row(pid: u32, stat: &str, clock: &Clock) -> Option<(Row, u64)> {
    let (head, fields) = stat.rsplit_once(") ")?;
    let command = head.split_once(" (")?.1.to_owned();
    let fields = fields.split_ascii_whitespace().collect::<Vec<_>>();
    let number = |index: usize| fields.get(index)?.parse::<u64>().ok();
    let (parent, user, system, start, resident) = (
        number(1)?,
        number(11)?,
        number(12)?,
        number(19)?,
        number(21)?,
    );
    let lived = clock.now.saturating_sub(start);
    let row = Row {
        pid,
        parent: u32::try_from(parent).ok()?,
        rss: resident.checked_mul(clock.page)?,
        // Tenths of a percent of one CPU over its life, as `ps` gives `pcpu`.
        cpu: (user + system)
            .checked_mul(1000)?
            .checked_div(lived)
            .unwrap_or(0),
        age_ms: lived.checked_mul(1000)? / clock.hertz,
        birth: start.to_string(),
        sampled_at_ms: clock.sampled_at_ms,
        command,
    };
    Some((row, start))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stat_line_reads_as_its_row() {
        let clock = Clock {
            hertz: 100,
            page: 4096,
            now: 3000,
            sampled_at_ms: 7,
        };
        let line = "4242 (tmux: server (1)) S 17 4242 4242 0 -1 4194560 120 0 0 0 250 50 0 0 20 0 1 0 1000 9000000 300 18446744073709551615";
        let (row, start) = row(4242, line, &clock).unwrap();
        assert_eq!(
            (
                row.pid,
                row.parent,
                row.command.as_str(),
                row.rss,
                row.cpu,
                row.age_ms,
                row.birth.as_str(),
                row.sampled_at_ms,
                start
            ),
            (
                4242,
                17,
                "tmux: server (1)",
                300 * 4096,
                150,
                20_000,
                "1000",
                7,
                1000
            )
        );
        assert!(super::row(1, "1 (init) S", &clock).is_none());
    }

    /// A process the table already read is given to another parent when its
    /// parent ends; the next read shows that parent, not the one it had.
    #[test]
    fn a_process_whose_parent_ended_is_read_again() {
        use std::io::{BufRead as _, Write as _};
        use std::process::Stdio;
        let mut shell = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & echo $!; read _"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(shell.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let sleep: u32 = line.trim().parse().unwrap();
        let parent_of = |rows: &[Row]| {
            rows.iter()
                .find(|row| row.pid == sleep)
                .map(|row| row.parent)
        };
        let mut table = Table::default();
        let before = parent_of(&table.read().unwrap());
        let _ = shell.stdin.take().unwrap().write_all(b"\n");
        let _ = shell.wait();
        let after = parent_of(&table.read().unwrap());
        // SAFETY: kill only signals the sleep this test started.
        unsafe { libc::kill(sleep as i32, libc::SIGKILL) };
        assert_eq!(before, Some(shell.id()));
        assert!(
            after.is_some_and(|parent| parent != shell.id()),
            "{after:?}"
        );
    }
}
