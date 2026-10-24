//! Claude print-mode runs left behind by a kernel that died without cleaning up.
//!
//! A Claude run is started in its own session, so a killed kernel does not take
//! it along, and nothing can ever read its output again. A new kernel ends
//! them rather than adopting them: the pipes that drove the run died with the
//! old kernel.
//!
//! A run names its kernel in the runtime files directory it was started with
//! (`<temp dir>/chariox-claude-remote-native-<kernel pid>-<random>`). It is
//! left behind when that pid is gone.

use std::collections::BTreeSet;
use std::path::Path;

use crate::provider::CLAUDE_RUNTIME_FILES_PREFIX;

const PRINT_MODE_MARKER: &str = "--input-format stream-json";

/// Pids of Claude print-mode runs started from `temp_dir` by a kernel whose pid
/// is not in `ps_output` (`pid= ppid= pgid= etimes= command=` lines).
pub(super) fn claude_runs_of_dead_kernels(
    ps_output: &str,
    current_pid: u32,
    temp_dir: &Path,
) -> Vec<u32> {
    let live: BTreeSet<u32> = ps_output
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect();
    let runtime_files = format!("{}/{CLAUDE_RUNTIME_FILES_PREFIX}", temp_dir.display());
    ps_output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid: u32 = parts.next()?.parse().ok()?;
            let mut command = parts.skip(3);
            let program = command.next()?;
            let command = command.collect::<Vec<_>>().join(" ");
            if Path::new(program).file_name()? != "claude" || !command.contains(PRINT_MODE_MARKER) {
                return None;
            }
            let owner = command
                .split_once(&runtime_files)?
                .1
                .split('-')
                .next()?
                .parse::<u32>()
                .ok()?;
            (owner != current_pid && !live.contains(&owner)).then_some(pid)
        })
        .collect()
}

/// A pid is only signalled when it shares this kernel's pid and mount
/// namespaces and its user: otherwise the owner pid in its command line may
/// belong to a kernel this one cannot see.
#[cfg(target_os = "linux")]
pub(super) fn shares_scope_with_this_kernel(pid: u32) -> bool {
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

#[cfg(not(target_os = "linux"))]
pub(super) fn shares_scope_with_this_kernel(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(pid: u32, ppid: u32, temp: &str, owner: u32) -> String {
        format!(
            "{pid} {ppid} {pid} 900 /root/.local/bin/claude -p --input-format stream-json --output-format stream-json --mcp-config {temp}/{CLAUDE_RUNTIME_FILES_PREFIX}{owner}-00ab/mcp.json --model x\n"
        )
    }

    #[test]
    fn a_run_whose_kernel_is_gone_is_left_behind() {
        let ps = format!(
            "50 1 50 5000 chariox-kernel\n{}{}{}{}",
            claude(100, 1, "/tmp", 40),   // owner 40 is gone
            claude(101, 50, "/tmp", 50),  // owner 50 is alive
            claude(102, 1, "/tmp", 60),   // this kernel's own run (60 = current)
            claude(103, 1, "/other", 40)  // another temp dir: not this kernel's
        );
        assert_eq!(
            claude_runs_of_dead_kernels(&ps, 60, Path::new("/tmp")),
            [100]
        );
    }

    #[test]
    fn only_claude_print_mode_runs_with_a_runtime_files_directory_are_taken() {
        let ps = "200 1 200 900 claude --mcp-config /tmp/chariox-claude-remote-native-40-ab/m.json\n\
                  201 1 201 900 vim /tmp/chariox-claude-remote-native-40-ab/notes --input-format stream-json\n\
                  202 1 202 900 claude -p --input-format stream-json --session-id x\n";
        assert!(claude_runs_of_dead_kernels(ps, 60, Path::new("/tmp")).is_empty());
    }
}
