//! A hot reload keeps the processes agents watch: the next program supervises
//! the same process under the same wake, so its exit or match still wakes the
//! owner. One it cannot supervise again ends its wake with a typed reason.
use super::*;
use serde::{Deserialize, Serialize};
use std::os::fd::{AsRawFd, FromRawFd as _};

/// What the next program needs to know about a watched process besides the
/// wake itself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct Origin {
    /// The agent's unit: the memory ceiling the process counts under.
    pub unit: String,
    pub session_id: String,
    /// The agent that lists the process in Health.
    pub agent: Option<String>,
    /// The room whose secrets scrub the output.
    pub protect_room: String,
}

/// One watched process handed to the next kernel program.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WatchedHandover {
    wake: String,
    pid: u32,
    birth: u64,
    /// The read ends of its output pipes, which the next program inherits.
    fds: Vec<i32>,
    origin: Origin,
    /// The memory domain the next program takes over.
    cgroup: Option<std::path::PathBuf>,
    output: OutputSnapshot,
}

impl WatchedHandover {
    pub(crate) fn fds(&self) -> &[i32] {
        &self.fds
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }
}

/// How long supervision may take to stop for a hot reload.
const STOP_LIMIT: Duration = Duration::from_secs(5);

impl WatchedProcesses {
    /// Stops supervising every running process without an outcome and gives
    /// the next program what it needs to supervise them again, with the pipes
    /// it keeps open until then. One that settled or did not stop in time is
    /// left out: the next program reports it lost.
    pub(crate) fn hand_over(&self) -> Vec<(WatchedHandover, Vec<OwnedFd>)> {
        self.stopping.store(true, Ordering::Release);
        let live: Vec<Owned> = self
            .live
            .lock()
            .map(|live| live.values().cloned().collect())
            .unwrap_or_default();
        for child in &live {
            if let Ok(mut child) = child.lock() {
                child.handing_over = true;
            }
        }
        let deadline = Instant::now() + STOP_LIMIT;
        while Instant::now() < deadline
            && live.iter().any(|child| {
                child
                    .lock()
                    .is_ok_and(|c| c.handed_over.is_none() && c.status.is_none() && !c.lost)
            })
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        live.iter()
            .filter_map(|child| {
                let mut child = child.lock().ok()?;
                if child.status.is_some() || child.lost {
                    return None;
                }
                let output = child.handed_over.take()?;
                let pipes = std::mem::take(&mut child.pipes);
                let handover = WatchedHandover {
                    wake: child.wake.clone(),
                    pid: child.child.id(),
                    birth: child.birth,
                    fds: pipes.iter().map(AsRawFd::as_raw_fd).collect(),
                    origin: child.origin.clone(),
                    cgroup: child
                        .resources
                        .as_ref()
                        .and_then(|domain| domain.cgroup_path()),
                    output,
                };
                Some((handover, pipes))
            })
            .collect()
    }
}

impl KernelRuntimeState {
    /// The first pass after a hot reload: supervises again each process the
    /// previous program handed over, under its wake. One that is gone ends its
    /// wake with `watch_lost_on_reload`; it is never relaunched.
    pub(in crate::runtime::state) fn adopt_watched_processes(
        &self,
        handed_over: Vec<WatchedHandover>,
        now: u64,
    ) -> Result<(), DaemonError> {
        if handed_over.is_empty() {
            return Ok(());
        }
        let wakes = self.owned.durable_state_store.agent_wakes(None, None)?;
        for handover in handed_over {
            // SAFETY: the previous program left these descriptors open for this
            // one alone, and nothing else owns them.
            let streams: Vec<OwnedFd> = handover
                .fds
                .iter()
                .map(|fd| unsafe { OwnedFd::from_raw_fd(*fd) })
                .collect();
            let Some(wake) = wakes.iter().find(|wake| {
                wake.id == handover.wake
                    && wake.kind == "process"
                    && matches!(wake.state.as_str(), "starting" | "running")
            }) else {
                crate::logging::warn_with_fields(
                    "agent_process_watch.handover",
                    "a handed-over process has no running wake; it is not supervised",
                    serde_json::json!({"wake": handover.wake, "pid": handover.pid}),
                );
                continue;
            };
            let same_process = super::super::agent_process_group::birth(handover.pid)
                .is_ok_and(|birth| birth == handover.birth);
            let kept = streams
                .iter()
                .map(OwnedFd::try_clone)
                .collect::<std::io::Result<Vec<_>>>();
            match (same_process, kept) {
                (true, Ok(kept)) => self.supervise_again(wake, handover, streams, kept),
                _ => {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::ProcessExited {
                            id: wake.id.clone(),
                            exit_code: None,
                            tail: format!(
                                "watch_lost_on_reload: process {} gone, exit unknown",
                                handover.pid
                            ),
                            now,
                        })?;
                    self.wake_notice(wake, format!("Wake alert: watched process '{}' could not be watched again after a kernel hot reload; its exit is unknown", wake.label));
                    self.schedule_wake_delivery(wake);
                }
            }
        }
        Ok(())
    }

    fn supervise_again(
        &self,
        wake: &AgentWake,
        handover: WatchedHandover,
        streams: Vec<OwnedFd>,
        kept: Vec<OwnedFd>,
    ) {
        let WatchedHandover {
            pid,
            birth,
            origin,
            cgroup,
            output,
            ..
        } = handover;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let output = Arc::new(Output::restored(output));
        let readers =
            self.drain_streams(streams, &origin, wake.match_text.clone(), &output, &sender);
        let mut child = OwnedChild::adopted(pid, birth, &wake.id, origin.clone(), kept);
        child.resources = match crate::runtime::process_resources::PreparedDomain::adopted(
            &origin.unit,
            cgroup.as_deref(),
        ) {
            Ok(resources) => watch_resources(resources, pid, &wake.id, &origin),
            Err(error) => {
                crate::logging::error_with_fields(
                    "agent_process_watch.handover",
                    "a handed-over process's resource monitoring could not start",
                    serde_json::json!({"error": error.to_string()}),
                );
                None
            }
        };
        let owned: Owned = Arc::new(std::sync::Mutex::new(child));
        let activity = self.owned.begin_managed_activity_mutation();
        if let Ok(mut live) = self.owned.agent_wakes.processes.live.lock() {
            live.insert(wake.id.clone(), owned.clone());
        }
        activity.record();
        supervise(owned, readers, output, sender);
        self.deliver_process_signals(
            wake.id.clone(),
            wake.room_id.clone(),
            wake.agent_id.clone(),
            receiver,
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::super::tests::{yield_on_watches, YieldedWatches};
    use super::*;

    /// A process watched when the kernel hot-reloads, as the next program
    /// finds it: handed over with its pipes, which the exec keeps open, and
    /// none of the previous program's supervision.
    async fn after_reload(argv: &[&str]) -> (YieldedWatches, Vec<WatchedHandover>) {
        let rig = yield_on_watches(vec![argv.iter().map(|a| a.to_string()).collect()]).await;
        let processes = &rig.state.owned.agent_wakes.processes;
        let (handed_over, pipes): (Vec<_>, Vec<_>) = processes.hand_over().into_iter().unzip();
        for pipe in pipes.into_iter().flatten() {
            std::mem::forget(pipe);
        }
        // The exec ends the previous program without dropping its state, which
        // would end the process with its resource domain.
        std::mem::forget(std::mem::take(&mut *processes.live.lock().unwrap()));
        processes.stopping.store(false, Ordering::Release);
        (rig, handed_over)
    }

    #[tokio::test]
    async fn a_hot_reload_keeps_the_wake_of_a_watched_process() {
        crate::test_support::isolated_env_test!();
        let (rig, handed_over) =
            after_reload(&["sh", "-c", "echo before; sleep 2; echo after; exit 7"]).await;
        assert_eq!(handed_over.len(), 1, "the running process is handed over");
        rig.state
            .adopt_watched_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        let wake = &rig.wakes[0];
        let answer = rig
            .inbox_event(|event| {
                let answer = &event.payload["public_answer"];
                (answer["wake_id"] == wake.id.as_str()).then(|| answer.clone())
            })
            .await;
        assert_eq!(answer["exit_code"], 7, "{answer}");
        let tail = answer["output_tail"].as_str().unwrap();
        assert!(
            tail.contains("before") && tail.contains("after"),
            "the output from both programs is in the tail: {tail:?}"
        );
        assert!(!tail.contains("process_lost"), "{tail:?}");
    }

    #[tokio::test]
    async fn a_watched_process_gone_in_a_hot_reload_wakes_its_owner_with_the_reason() {
        crate::test_support::isolated_env_test!();
        let (rig, handed_over) = after_reload(&["sleep", "60"]).await;
        let pid = handed_over[0].pid();
        // Something else ends and reaps it while the kernel swaps programs.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
            libc::waitpid(pid as i32, std::ptr::null_mut(), 0);
        }
        rig.state
            .adopt_watched_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        let tail = rig
            .inbox_event(|event| {
                event.payload["public_answer"]["output_tail"]
                    .as_str()
                    .map(str::to_owned)
            })
            .await;
        assert_eq!(
            tail,
            format!("watch_lost_on_reload: process {pid} gone, exit unknown")
        );
    }
}
