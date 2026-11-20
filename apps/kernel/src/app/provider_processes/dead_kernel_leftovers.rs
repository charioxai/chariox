//! End abandoned provider work only when its launching kernel instance is gone.

#[cfg(target_os = "linux")]
use crate::provider::process_owner::{boot_id, home_identity, OWNER_ENV};
#[cfg(target_os = "linux")]
use crate::runtime::owned_process_signals::{linux_ancestors, linux_start, FrozenLinuxProcess};

#[cfg(target_os = "linux")]
fn owner<'a>(environ: &'a [u8], boot: &str, namespace: &str, home: &str) -> Option<(u32, &'a str)> {
    let marker = environ.split(|byte| *byte == 0).find_map(|entry| {
        let (key, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
        (key == OWNER_ENV).then_some(value)
    })?;
    let (pid, identity) = marker.split_once(':')?;
    let (start, scope) = identity.split_once(':')?;
    let (owner_boot, scope) = scope.split_once(':')?;
    let (owner_namespace, owner_home) = scope.rsplit_once(':')?;
    let pid = pid.parse().ok()?;
    (pid > 1
        && start.parse::<u64>().ok()? > 0
        && owner_boot == boot
        && owner_namespace == namespace
        && owner_home == home)
        .then_some((pid, start))
}

#[cfg(target_os = "linux")]
struct Candidate {
    pid: u32,
    start: String,
    owner_pid: u32,
}

#[cfg(target_os = "linux")]
pub(super) fn stop() -> usize {
    use std::os::unix::fs::MetadataExt as _;
    let (Ok(namespace), Ok(me), Some(boot), Some(home), Ok(ancestors)) = (
        std::fs::read_link("/proc/self/ns/pid"),
        std::fs::metadata("/proc/self"),
        boot_id(),
        home_identity(),
        linux_ancestors(),
    ) else {
        return 0;
    };
    let Some(namespace_name) = namespace.to_str() else {
        return 0;
    };
    let result = reap(&namespace, || {
        let candidates = std::fs::read_dir("/proc")?
            .flatten()
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                if pid <= 1
                    || ancestors.contains(&pid)
                    || !entry.metadata().is_ok_and(|m| m.uid() == me.uid())
                {
                    return None;
                }
                let path = entry.path();
                // The launch PID has meaning only in the owner's PID namespace.
                // A restarted systemd service may have a new mount namespace.
                if std::fs::read_link(path.join("ns/pid")).ok()? != namespace {
                    return None;
                }
                let start = linux_start(pid).ok()??;
                let environ = std::fs::read(path.join("environ")).ok()?;
                let (owner_pid, owner_start) = owner(&environ, boot, namespace_name, &home)?;
                if linux_start(owner_pid).ok()?.as_deref() == Some(owner_start) {
                    return None;
                }
                Some(Candidate {
                    pid,
                    start,
                    owner_pid,
                })
            })
            .collect();
        Ok(candidates)
    });
    result.unwrap_or_else(|error| {
        crate::logging::warn_with_fields(
            "daemon.provider_process_gc",
            "could not finish reaping processes left by dead kernels",
            serde_json::json!({"error": error.to_string()}),
        );
        0
    })
}

#[cfg(target_os = "linux")]
fn reap(
    namespace: &std::path::Path,
    mut discover: impl FnMut() -> std::io::Result<Vec<Candidate>>,
) -> std::io::Result<usize> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = std::collections::BTreeSet::new();
    let mut frozen = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(
                "orphan provider processes did not settle",
            ));
        }
        let mut new = false;
        for Candidate {
            pid,
            start,
            owner_pid,
        } in discover()?
        {
            if !seen.insert((pid, start.clone())) {
                continue;
            }
            new = true;
            if let Some(process) = FrozenLinuxProcess::stop(pid, &start)? {
                frozen.push((pid, owner_pid, process));
            }
        }
        if !new {
            break;
        }
        // A waiting shell must stop before any owned child is killed. Once all
        // threads are stopped, rescan for work they forked during the sweep.
        loop {
            let mut quiescent = true;
            for (_, _, process) in &frozen {
                quiescent &= process.is_quiescent(namespace)?;
            }
            if quiescent {
                break;
            }
            if Instant::now() >= deadline {
                // Another sweep can finish these processes. Resuming a shell
                // here could race with another kernel reaping the same owner.
                return Err(std::io::Error::other(
                    "orphan provider processes did not freeze",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let mut killed = 0;
    for (pid, owner_pid, process) in frozen {
        match process.kill() {
            Ok(()) => {
                killed += 1;
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
    Ok(killed)
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
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:boot:pid:[123]:home-a",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            Some((42, "123"))
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_PID=42",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:old_boot:pid:[123]:home-a",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:0:boot:pid:[123]:home-a",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:boot:pid:[999]:home-a",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            None
        );
        assert_eq!(
            owner(
                b"CHARIOX_PROVIDER_OWNER_INSTANCE=42:123:boot:pid:[123]:home-b",
                "boot",
                "pid:[123]",
                "home-a"
            ),
            None
        );
    }

    #[test]
    fn child_first_reap_never_releases_a_waiting_shell_to_fork_again() {
        use crate::runtime::bounded_process::BoundedChildExt as _;
        use crate::runtime::owned_process_signals::OwnedProcessSignals;
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Stdio};

        struct Fixture {
            child: Child,
            cleanup: OwnedProcessSignals,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.cleanup.kill_owned_processes();
                let _ = self.child.bounded_wait();
            }
        }

        let root = tempfile::tempdir().unwrap();
        let child = crate::process_spawn::command("sh")
            .args([
                "-c",
                r#"sleep 600 & child=$!; echo "$child"; wait "$child"; touch MARKER; sleep 600 & echo "$!" > FORK; wait"#,
            ])
            .current_dir(root.path())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let cleanup = OwnedProcessSignals::for_child(&child).unwrap();
        let mut fixture = Fixture { child, cleanup };
        let parent_pid = fixture.child.id();
        let mut line = String::new();
        BufReader::new(fixture.child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let child_pid: u32 = line.trim().parse().unwrap();
        let birth = |pid| linux_start(pid).unwrap().unwrap();
        let parent_start = birth(parent_pid);
        let child_start = birth(child_pid);
        let namespace = std::fs::read_link("/proc/self/ns/pid").unwrap();

        let mut scan = 0;
        let killed = reap(&namespace, || {
            scan += 1;
            Ok(match scan {
                1 => vec![Candidate {
                    pid: child_pid,
                    start: child_start.clone(),
                    owner_pid: 2,
                }],
                2 => {
                    // The parent is discovered on a subsequent scan. A child
                    // killed eagerly would release the shell during this gap.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    vec![Candidate {
                        pid: parent_pid,
                        start: parent_start.clone(),
                        owner_pid: 2,
                    }]
                }
                _ => Vec::new(),
            })
        })
        .unwrap();

        assert_eq!(killed, 2);
        assert!(scan >= 3);
        assert!(!fixture.child.bounded_wait().unwrap().success());
        assert!(!root.path().join("MARKER").exists());
        assert!(!root.path().join("FORK").exists());
    }
}
