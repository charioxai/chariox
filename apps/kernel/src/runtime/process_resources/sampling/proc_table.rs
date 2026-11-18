//! The process table as Linux keeps it in /proc: one stat file per process, read
//! on the sampler's own thread. Starting `ps` five times a second cost the kernel
//! 11-14 s of CPU a minute with 12 watched commands on a host of 2800
//! processes; reading /proc here costs about half of that.
use super::{shortage, Row, SamplingError};

pub(super) fn read() -> Result<Vec<Row>, SamplingError> {
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
    let mut rows = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(unreadable)? {
        let Some(pid) = entry
            .map_err(unreadable)?
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let stat = match std::fs::read(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            // It ended after it was listed, or this kernel may not read it.
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT | libc::ESRCH | libc::EACCES | libc::EPERM)
                ) =>
            {
                continue
            }
            Err(error) => return Err(unreadable(error)),
        };
        rows.extend(row(
            pid,
            &String::from_utf8_lossy(&stat),
            now,
            hertz,
            page,
            sampled_at_ms,
        ));
    }
    if rows.is_empty() {
        return Err(SamplingError::Unusable(
            "process resource monitor read an empty process table from /proc".into(),
        ));
    }
    Ok(rows)
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
fn row(pid: u32, stat: &str, now: u64, hertz: u64, page: u64, sampled_at_ms: u64) -> Option<Row> {
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
    let lived = now.saturating_sub(start);
    Some(Row {
        pid,
        parent: u32::try_from(parent).ok()?,
        rss: resident.checked_mul(page)?,
        // Tenths of a percent of one CPU over its life, as `ps` gives `pcpu`.
        cpu: (user + system)
            .checked_mul(1000)?
            .checked_div(lived)
            .unwrap_or(0),
        age_ms: lived.checked_mul(1000)? / hertz,
        birth: start.to_string(),
        sampled_at_ms,
        command,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_stat_line_reads_as_its_row() {
        let line = "4242 (tmux: server (1)) S 17 4242 4242 0 -1 4194560 120 0 0 0 250 50 0 0 20 0 1 0 1000 9000000 300 18446744073709551615";
        let row = super::row(4242, line, 3000, 100, 4096, 7).unwrap();
        assert_eq!(
            (
                row.pid,
                row.parent,
                row.command.as_str(),
                row.rss,
                row.cpu,
                row.age_ms,
                row.birth.as_str(),
                row.sampled_at_ms
            ),
            (
                4242,
                17,
                "tmux: server (1)",
                300 * 4096,
                150,
                20_000,
                "1000",
                7
            )
        );
        assert!(super::row(1, "1 (init) S", 3000, 100, 4096, 7).is_none());
    }
}
