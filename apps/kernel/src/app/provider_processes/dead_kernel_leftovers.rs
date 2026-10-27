//! Processes a dead kernel left behind.
//!
//! A provider run that outlives its kernel is debris: its pipes died with the kernel, so nothing
//! can read its output again. Its tool processes are worse. The restored kernel resumes the
//! interrupted turn, the provider notices the missing result and runs the command again, and two
//! copies of a build or a commit run at once.
//!
//! Every process a kernel launches for a provider run names that kernel's pid in its
//! environment, and the processes the run starts inherit it. The processes whose kernel pid is
//! gone are the ones to end. A run started by a kernel that predates the variable names its
//! kernel in its Claude runtime files directory instead
//! (`<temp dir>/chariox-claude-remote-native-<kernel pid>-<random>`).

use std::path::Path;

use crate::provider::{CLAUDE_RUNTIME_FILES_PREFIX, PROVIDER_OWNER_PID_ENV};

const CLAUDE_EVENTS_FILE_ENV: &str = "CHARIOX_CLAUDE_NATIVE_EVENTS";

/// The kernel pid a process's environment (`NAME=value` entries separated by NUL) names.
pub(super) fn owner_pid_of(environ: &[u8], temp_dir: &Path) -> Option<u32> {
    let runtime_files = format!("{}/{CLAUDE_RUNTIME_FILES_PREFIX}", temp_dir.display());
    let mut from_runtime_files = None;
    for entry in environ.split(|byte| *byte == 0) {
        let Ok(entry) = std::str::from_utf8(entry) else {
            continue;
        };
        match entry.split_once('=') {
            Some((PROVIDER_OWNER_PID_ENV, pid)) => return pid.parse().ok(),
            Some((CLAUDE_EVENTS_FILE_ENV, events_file)) => {
                from_runtime_files = events_file
                    .strip_prefix(&runtime_files)
                    .and_then(|rest| rest.split('-').next()?.parse().ok());
            }
            _ => {}
        }
    }
    from_runtime_files
}

/// The processes of `(pid, kernel pid)` launches whose kernel is neither this one nor alive.
pub(super) fn left_by_dead_kernels(
    launches: impl IntoIterator<Item = (u32, u32)>,
    current_pid: u32,
    is_alive: impl Fn(u32) -> bool,
) -> Vec<u32> {
    launches
        .into_iter()
        .filter(|(pid, owner)| {
            *pid > 1
                && *pid != current_pid
                && *owner > 1
                && *owner != current_pid
                && !is_alive(*owner)
        })
        .map(|(pid, _)| pid)
        .collect()
}

/// Every process this kernel may signal that a kernel launched, with that kernel's pid. A pid is
/// only considered when it shares this kernel's pid and mount namespaces and its user: otherwise
/// the kernel pid in its environment may belong to a kernel this one cannot see.
#[cfg(target_os = "linux")]
pub(super) fn launches_in_this_scope(temp_dir: &Path) -> Vec<(u32, u32)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
            Some((pid, owner_pid_of(&environ, temp_dir)?))
        })
        .filter(|(pid, _)| shares_scope_with_this_kernel(*pid))
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn launches_in_this_scope(_temp_dir: &Path) -> Vec<(u32, u32)> {
    Vec::new()
}

#[cfg(target_os = "linux")]
pub(super) fn is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn is_alive(_pid: u32) -> bool {
    true
}

#[cfg(target_os = "linux")]
fn shares_scope_with_this_kernel(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let same = |name: &str| {
        matches!(
            (
                std::fs::read_link(format!("/proc/self/ns/{name}")),
                std::fs::read_link(format!("/proc/{pid}/ns/{name}")),
            ),
            (Ok(mine), Ok(theirs)) if mine == theirs
        )
    };
    let same_user = matches!(
        (std::fs::metadata("/proc/self"), std::fs::metadata(format!("/proc/{pid}"))),
        (Ok(mine), Ok(theirs)) if mine.uid() == theirs.uid()
    );
    same("pid") && same("mnt") && same_user
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environ(entries: &[&str]) -> Vec<u8> {
        entries.join("\0").into_bytes()
    }

    #[test]
    fn a_process_names_its_kernel_in_the_launch_variable() {
        let environ = environ(&[
            "PATH=/bin",
            &format!("{PROVIDER_OWNER_PID_ENV}=4242"),
            "HOME=/x",
        ]);
        assert_eq!(owner_pid_of(&environ, Path::new("/tmp")), Some(4242));
    }

    #[test]
    fn a_run_from_before_the_variable_names_its_kernel_in_its_runtime_files_directory() {
        let environ = environ(&[&format!(
            "{CLAUDE_EVENTS_FILE_ENV}=/tmp/{CLAUDE_RUNTIME_FILES_PREFIX}77-00ab/events.jsonl"
        )]);
        assert_eq!(owner_pid_of(&environ, Path::new("/tmp")), Some(77));
        assert_eq!(
            owner_pid_of(&environ, Path::new("/elsewhere")),
            None,
            "another temp dir is not this kernel's"
        );
    }

    #[test]
    fn a_process_that_no_kernel_launched_has_no_owner() {
        let environ = environ(&["PATH=/bin", "CHARIOX_PROVIDER_OWNER_PID=not-a-pid"]);
        assert_eq!(owner_pid_of(&environ, Path::new("/tmp")), None);
        assert_eq!(owner_pid_of(b"", Path::new("/tmp")), None);
    }

    #[test]
    fn only_processes_of_a_dead_kernel_are_left_behind() {
        let launches = [
            (100, 40),
            (101, 50),
            (102, 60),
            (103, 40),
            (1, 40),
            (104, 1),
        ];
        // 40 is gone, 50 is alive, 60 is this kernel, pid 1 and kernel 1 are never touched.
        let left = left_by_dead_kernels(launches, 60, |kernel| kernel == 50);
        assert_eq!(left, [100, 103]);
    }
}
