//! End abandoned provider work only when its launching kernel instance is gone.

#[cfg(target_os = "linux")]
use crate::provider::process_owner::{boot_id, OWNER_ENV};
#[cfg(target_os = "linux")]
use crate::runtime::owned_process_signals::{
    kill_linux_process_with_start, linux_ancestors, linux_start,
};

#[cfg(target_os = "linux")]
fn owner<'a>(environ: &'a [u8], boot: &str, namespace: &str) -> Option<(u32, &'a str)> {
    let marker = environ.split(|byte| *byte == 0).find_map(|entry| {
        let (key, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
        (key == OWNER_ENV).then_some(value)
    })?;
    let (pid, identity) = marker.split_once(':')?;
    let (start, scope) = identity.split_once(':')?;
    let (owner_boot, owner_namespace) = scope.split_once(':')?;
    let pid = pid.parse().ok()?;
    (pid > 1
        && start.parse::<u64>().ok()? > 0
        && owner_boot == boot
        && owner_namespace == namespace)
        .then_some((pid, start))
}

#[cfg(target_os = "linux")]
pub(super) fn stop() -> usize {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    let (Ok(pid_ns), Ok(me), Some(boot)) = (
        std::fs::read_link("/proc/self/ns/pid"),
        std::fs::metadata("/proc/self"),
        boot_id(),
    ) else {
        return 0;
    };
    let Some(namespace) = pid_ns.to_str() else {
        return 0;
    };
    let Ok(ancestors) = linux_ancestors() else {
        return 0;
    };
    let mut stopped = 0;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid <= 1
            || ancestors.contains(&pid)
            || !entry.metadata().is_ok_and(|m| m.uid() == me.uid())
        {
            continue;
        }
        let path = entry.path();
        // The recorded PID has meaning only in the owner's PID namespace.
        // A restarted systemd service may have a new mount namespace.
        if !std::fs::read_link(path.join("ns/pid")).is_ok_and(|ns| ns == pid_ns) {
            continue;
        }
        let Ok(Some(start)) = linux_start(pid) else {
            continue;
        };
        let Ok(environ) = std::fs::read(path.join("environ")) else {
            continue;
        };
        let Some((owner_pid, owner_start)) = owner(&environ, boot, namespace) else {
            continue;
        };
        if !linux_start(owner_pid).is_ok_and(|live| live.as_deref() != Some(owner_start)) {
            continue;
        }
        match kill_linux_process_with_start(pid, &start) {
            Ok(()) => {
                stopped += 1;
                crate::logging::warn_with_fields(
                    "daemon.provider_process_gc",
                    "reaped provider process left by a dead kernel",
                    serde_json::json!({"pid": pid, "owner_pid": owner_pid}),
                );
            }
            Err(error) => crate::logging::warn_with_fields(
                "daemon.provider_process_gc",
                "could not reap provider process left by a dead kernel",
                serde_json::json!({"pid": pid, "owner_pid": owner_pid, "error": error.to_string()}),
            ),
        }
    }
    stopped
}

#[cfg(not(target_os = "linux"))]
pub(super) fn stop() -> usize {
    0
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn provider_owner_must_include_start_time_and_current_boot() {
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:boot:pid:[123]",
                "boot",
                "pid:[123]"
            ),
            Some((42, "123"))
        );
        assert_eq!(
            owner(b"CHARIOX_PROVIDER_OWNER_PID=42", "boot", "pid:[123]"),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:old_boot:pid:[123]",
                "boot",
                "pid:[123]"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:0:boot:pid:[123]",
                "boot",
                "pid:[123]"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:boot:pid:[999]",
                "boot",
                "pid:[123]"
            ),
            None
        );
    }
}
