use super::{Result, WorkerError, MEMORY_BYTES, MINIMUM_KERNEL, TASKS};
use std::collections::BTreeMap;

pub(super) fn number(value: &str) -> Result<u64> {
    let value = value.strip_suffix('\n').unwrap_or(value);
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WorkerError::ResourceDomain);
    }
    let number = value
        .parse::<u64>()
        .map_err(|_| WorkerError::ResourceDomain)?;
    if number.to_string() != value {
        return Err(WorkerError::ResourceDomain);
    }
    Ok(number)
}

pub(super) fn fields(value: &str, separator: char) -> Result<BTreeMap<&str, &str>> {
    if value.len() > 32768 {
        return Err(WorkerError::ResourceDomain);
    }
    let mut fields = BTreeMap::new();
    for line in value.lines() {
        let (key, value) = line
            .split_once(separator)
            .ok_or(WorkerError::ResourceDomain)?;
        if key.is_empty()
            || key.len() > 64
            || fields.len() >= 128
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || fields.insert(key, value.trim()).is_some()
        {
            return Err(WorkerError::ResourceDomain);
        }
    }
    Ok(fields)
}

pub(super) fn limits(values: &BTreeMap<&str, String>) -> Result<()> {
    for (name, expected) in [
        ("memory.max", MEMORY_BYTES),
        ("memory.swap.max", 0),
        ("memory.oom.group", 1),
        ("pids.max", TASKS),
    ] {
        if number(values.get(name).ok_or(WorkerError::ResourceDomain)?)? != expected {
            return Err(WorkerError::ResourceDomain);
        }
    }
    if values.get("cpu.max").map(|v| v.trim()) != Some("100000 100000") {
        return Err(WorkerError::ResourceDomain);
    }
    Ok(())
}

pub(super) fn pids(value: &str) -> Result<Vec<i32>> {
    if value.len() > 4096 {
        return Err(WorkerError::ResourceDomain);
    }
    let mut pids = Vec::new();
    for line in value.lines() {
        let pid = i32::try_from(number(line)?).map_err(|_| WorkerError::ResourceDomain)?;
        if pid <= 1 || pids.len() >= TASKS as usize || pids.contains(&pid) {
            return Err(WorkerError::ResourceDomain);
        }
        pids.push(pid);
    }
    Ok(pids)
}

pub(super) fn kernel(release: &str) -> Result<()> {
    let mut parts = release.trim().split('.');
    let major = number(parts.next().ok_or(WorkerError::Preparation)?)?;
    let minor = number(parts.next().ok_or(WorkerError::Preparation)?)?;
    if (major, minor) < MINIMUM_KERNEL {
        return Err(WorkerError::Preparation);
    }
    Ok(())
}

pub(super) fn worker_status(text: &str, pid: i32, parent: i32, uid: u32, gid: u32) -> Result<()> {
    let values = fields(text, ':')?;
    let field = |key| values.get(key).copied().ok_or(WorkerError::Identity);
    for (key, expected) in [
        ("Pid", pid as u64),
        ("PPid", parent as u64),
        ("NoNewPrivs", 1),
        ("Seccomp", 2),
    ] {
        if number(field(key)?)? != expected {
            return Err(WorkerError::Identity);
        }
    }
    for key in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
        if field(key)? != "0000000000000000" {
            return Err(WorkerError::Identity);
        }
    }
    for (key, expected) in [("Uid", uid), ("Gid", gid)] {
        let ids = field(key)?
            .split_whitespace()
            .map(number)
            .collect::<Result<Vec<_>>>()?;
        if ids != [expected as u64; 4] {
            return Err(WorkerError::Identity);
        }
    }
    if !field("Groups")?.is_empty() {
        return Err(WorkerError::Identity);
    }
    let nested = field("NSpid")?
        .split_whitespace()
        .map(number)
        .collect::<Result<Vec<_>>>()?;
    if nested.len() < 2
        || nested.len() > 32
        || nested.first() != Some(&(pid as u64))
        || nested.last() != Some(&1)
    {
        return Err(WorkerError::Identity);
    }
    Ok(())
}
