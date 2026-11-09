//! A disk running out of room (OF-39): idle temp folders are trimmed, and
//! harder while a volume is below its floor. The kernel deletes nothing but its
//! own scratch, decides nothing about content and sends no agent a prompt about
//! disk space: what fills a disk is often outside every agent.
use super::*;
use crate::runtime::process_resources;
use std::time::Instant;

const CHECK_EVERY: Duration = Duration::from_secs(5);
const TRIM_EVERY: Duration = Duration::from_secs(60);
/// Under pressure an item nothing wrote for this long goes, however long the
/// usual window is.
const PRESSURE_IDLE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Pace {
    trimmed: Option<Instant>,
}

/// A blocking syscall already in progress may finish after shutdown; it must
/// not begin another scan or trim once the owning maintenance task is gone.
struct ScanCancellation(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ScanCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn due(slot: &mut Option<Instant>, every: Duration) -> bool {
    if slot.is_some_and(|at| at.elapsed() < every) {
        return false;
    }
    *slot = Some(Instant::now());
    true
}

impl KernelRuntimeState {
    fn disk_volume_floor(&self) -> fn(&std::path::Path) -> Option<(u64, u64, u64)> {
        #[cfg(test)]
        if let Some(probe) = self.disk_volume_probe {
            return probe;
        }
        process_resources::volume_floor
    }

    /// One pass at a time, paced from completion rather than its start. A slow
    /// scan never accumulates more work.
    pub(super) async fn run_disk_maintenance(&self) {
        let mut pace = Pace::default();
        loop {
            self.tend_disk(&mut pace).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    }

    async fn tend_disk(&self, pace: &mut Pace) {
        let trim = due(&mut pace.trimmed, TRIM_EVERY);
        let state = self.clone();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_scan = ScanCancellation(cancelled.clone());
        // All filesystem work shares one awaited blocking job. In particular,
        // periodic trims cannot overlap the pressure scan or another pass.
        let scan = tokio::task::spawn_blocking(move || {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            if trim {
                Self::trim_temp(agent_temp_idle());
            }
            let volume_floor = state.disk_volume_floor();
            let pressured = state.volumes_below_floor(|path| {
                (!cancelled.load(Ordering::Acquire))
                    .then(|| volume_floor(path))
                    .flatten()
            });
            if !cancelled.load(Ordering::Acquire) && pressured {
                Self::trim_temp(PRESSURE_IDLE);
            }
        })
        .await;
        if let Err(error) = scan {
            tracing::warn!(%error, "disk maintenance scan failed; retrying next pass");
        }
    }

    fn trim_temp(idle: Duration) -> crate::agent_temp::Trimmed {
        let (Some(root), Some(held)) = (crate::agent_temp::root(), crate::agent_temp::held_paths())
        else {
            return Default::default();
        };
        crate::agent_temp::trim(&root, std::time::SystemTime::now(), idle, &held)
    }

    /// Whether a volume holding a live agent's workspace or scratch is below
    /// its floor.
    fn volumes_below_floor(
        &self,
        volume_floor: impl Fn(&std::path::Path) -> Option<(u64, u64, u64)>,
    ) -> bool {
        // Processes and scratch belong to the agent even after a run ends.
        self.owned
            .agent_store
            .list_agents()
            .into_iter()
            .any(|agent| {
                if agent.archived() || agent.remote_execution().is_some() {
                    return false;
                }
                let Ok(session) = self.owned.session_store.get_session(agent.session_id()) else {
                    return false;
                };
                let workspace =
                    std::path::PathBuf::from(agent.worktree_id().unwrap_or(session.worktree_id()));
                let temp = crate::agent_temp::dir(session.id(), agent.id());
                let short = [Some(workspace), temp]
                    .into_iter()
                    .flatten()
                    .filter_map(|path| volume_floor(&path))
                    .any(|(_, free, floor)| free < floor);
                short
            })
    }
}

fn agent_temp_idle() -> Duration {
    crate::agent_temp::idle_window()
}

#[cfg(test)]
mod tests;
