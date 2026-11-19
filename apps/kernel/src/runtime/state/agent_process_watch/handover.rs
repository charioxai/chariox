//! A hot reload keeps the processes agents watch: the next program supervises
//! the same process under the same wake, so its exit or match still wakes the
//! owner. One it cannot supervise again ends its wake with a typed reason.
//! A worker's leased process keeps its wake in its receipt, not in the wake
//! ledger, and is handed over the same way.
use super::*;
use crate::durable_state::agent_lifecycle::Outcome;
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

/// Owns what the old supervisor must keep alive until exec.
pub(crate) struct WatchedProcessTransfer {
    pub(crate) handover: WatchedHandover,
    pub(crate) pipes: Vec<OwnedFd>,
    pub(crate) resources: Option<crate::runtime::process_resources::ProcessDomain>,
}

/// How long supervision may take to stop for a hot reload.
const STOP_LIMIT: Duration = Duration::from_secs(5);

impl WatchedProcesses {
    /// Stops supervising every running process without an outcome and gives
    /// the next program what it needs to supervise them again, with the pipes
    /// it keeps open until then. One that settled or did not stop in time is
    /// left out: the next program reports it lost.
    pub(crate) fn hand_over(&self) -> Vec<WatchedProcessTransfer> {
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
                // Dropping the old resource watcher stops the command. The
                // prepared swap holds it even after the runtime is dropped.
                Some(WatchedProcessTransfer {
                    handover,
                    pipes,
                    resources: child.resources.take(),
                })
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
        for handover in handed_over {
            // SAFETY: the previous program left these descriptors open for this
            // one alone, and nothing else owns them.
            let streams: Vec<OwnedFd> = handover
                .fds
                .iter()
                .map(|fd| unsafe { OwnedFd::from_raw_fd(*fd) })
                .collect();
            let Some(wake) = self.supervised_wake(&handover.wake)? else {
                crate::logging::warn_with_fields(
                    "agent_process_watch.handover",
                    "a handed-over process has no armed wake; it is not supervised",
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
                (true, Ok(kept)) => self.supervise_again(&wake, handover, streams, kept),
                _ => {
                    let outcome = self.apply_agent_process_receipt(Operation::ProcessExited {
                        id: wake.id.clone(),
                        exit_code: None,
                        tail: format!(
                            "watch_lost_on_reload: process {} gone, exit unknown",
                            handover.pid
                        ),
                        now,
                    })?;
                    // A leased process's receipt tells only its home.
                    if let Outcome::Wakes(wakes) = outcome {
                        for wake in wakes {
                            self.wake_notice(&wake, format!("Wake alert: watched process '{}' could not be watched again after a kernel hot reload; its exit is unknown", wake.label));
                            self.schedule_wake_delivery(&wake);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The wake a handed-over process still answers to: one that is armed,
    /// including one being cancelled, in the wake ledger or, for a worker's
    /// leased process, in its receipt.
    fn supervised_wake(&self, id: &str) -> Result<Option<AgentWake>, DaemonError> {
        let store = &self.owned.durable_state_store;
        let wake = match store.leased_process_receipt(id)? {
            Some(receipt) => Some(receipt.wake),
            None => store
                .agent_wakes(None, None)?
                .into_iter()
                .find(|wake| wake.id == id),
        };
        Ok(wake.filter(|wake| {
            wake.kind == "process"
                && matches!(wake.state.as_str(), "starting" | "running" | "cancelling")
        }))
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
        // A match the previous program saw as it stopped may not have been
        // heard; the ledger counts a match once, so saying it again is safe.
        if let Some(line) = output.matched_line() {
            let _ = sender.send(Signal::Matched(line));
        }
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
        // The previous program's termination did not outlive it: ask again,
        // and force it after the grace period, until the process is gone.
        if wake.state == "cancelling" {
            self.owned.agent_wakes.processes.terminate(&wake.id);
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::super::tests::{yield_on_watches_with, WatchRig, YieldedWatches};
    use super::*;
    use crate::runtime::process_resources::{pause_sampling_for_key, ProcessDomain};

    #[derive(Default)]
    struct PreviousWatches {
        // Resume before dropping the resource guards, which join the watchers.
        sampling_pauses: Vec<std::sync::mpsc::Sender<()>>,
        resources: Vec<ProcessDomain>,
    }

    type Reloaded = (YieldedWatches, Vec<WatchedHandover>, PreviousWatches);

    /// A process watched when the kernel hot-reloads, as the next program
    /// finds it: handed over with its pipes, which the exec keeps open, and
    /// none of the previous program's supervision.
    async fn after_reload(argv: &[&str]) -> Reloaded {
        restarted(watching(argv, WatchRig::default()).await).await
    }

    async fn restarted(rig: YieldedWatches) -> Reloaded {
        let (rig, handed_over, resources) = reload(rig);
        next_program(&rig).await;
        (rig, handed_over, resources)
    }

    async fn watching(argv: &[&str], rig: WatchRig) -> YieldedWatches {
        let rig =
            yield_on_watches_with(vec![argv.iter().map(|a| a.to_string()).collect()], rig).await;
        // The process starts, prints and sets its signal handlers.
        tokio::time::sleep(Duration::from_millis(500)).await;
        rig
    }

    fn reload(rig: YieldedWatches) -> Reloaded {
        let processes = &rig.state.owned.agent_wakes.processes;
        let mut handed_over = Vec::new();
        let mut previous = PreviousWatches::default();
        for transfer in processes.hand_over() {
            for pipe in transfer.pipes {
                std::mem::forget(pipe);
            }
            if let Some(resources) = transfer.resources {
                // Exec ends the old watcher before the successor moves the
                // workload out of its cgroup. Park it in this in-process test.
                previous
                    .sampling_pauses
                    .push(pause_sampling_for_key(&transfer.handover.wake));
                previous.resources.push(resources);
            }
            handed_over.push(transfer.handover);
        }
        // The old runtime is dropped before exec. Only the resources owned by
        // the prepared swap may keep the process alive through that drop.
        processes.live.lock().unwrap().clear();
        (rig, handed_over, previous)
    }

    /// The previous program's delivery has stopped hearing signals.
    async fn next_program(rig: &YieldedWatches) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        rig.state
            .owned
            .agent_wakes
            .processes
            .stopping
            .store(false, Ordering::Release);
    }

    /// Ends a handed-over process still running when a test fails.
    struct KillOnDrop(u32, u64);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            // Signals only the session members whose birth still matches, so a reused pid is never hit.
            super::super::super::agent_process_group::signal_session(self.0, self.1, libc::SIGKILL);
        }
    }

    fn gone(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == -1 }
    }

    fn wake_state(rig: &YieldedWatches) -> String {
        rig.state
            .owned
            .durable_state_store
            .agent_wakes(Some(&rig.room), Some(&rig.agent))
            .unwrap()
            .into_iter()
            .find(|wake| wake.id == rig.wakes[0].id)
            .unwrap()
            .state
    }

    async fn wake_ends(rig: &YieldedWatches, state: &str) -> AgentWake {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let wake = rig
                .state
                .owned
                .durable_state_store
                .agent_wakes(Some(&rig.room), Some(&rig.agent))
                .unwrap()
                .into_iter()
                .find(|wake| wake.id == rig.wakes[0].id)
                .unwrap();
            if wake.state == state {
                return wake;
            }
            assert!(
                Instant::now() < deadline,
                "the wake never reached {state}: {wake:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn a_hot_reload_keeps_the_wake_of_a_watched_process() {
        crate::test_support::isolated_env_test!();
        let gate = crate::test_support::TestTempDir::new("watched-reload");
        let resume = gate.path().join("resume");
        let (rig, handed_over, _resources) = after_reload(&[
            "sh",
            "-c",
            "echo before; while [ ! -f \"$1\" ]; do sleep 0.05; done; echo after; exit 7",
            "watched-reload",
            resume.to_str().unwrap(),
        ])
        .await;
        assert_eq!(handed_over.len(), 1, "the running process is handed over");
        rig.state
            .adopt_watched_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        std::fs::write(resume, "").unwrap();
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
        let (rig, handed_over, _resources) = after_reload(&["sleep", "60"]).await;
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

    #[tokio::test]
    async fn a_leased_process_is_supervised_again_after_a_worker_reload_and_can_be_cancelled() {
        crate::test_support::isolated_env_test!();
        let leased = WatchRig {
            leased: true,
            ..Default::default()
        };
        let (rig, handed_over, _resources) =
            restarted(watching(&["sleep", "60"], leased).await).await;
        let (pid, birth) = (handed_over[0].pid(), handed_over[0].birth);
        let _kill = KillOnDrop(pid, birth);
        let id = rig.wakes[0].id.clone();
        rig.state
            .recover_worker_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        let store = &rig.state.owned.durable_state_store;
        let mut receipt = store.leased_process_receipt(&id).unwrap().unwrap();
        assert_eq!(
            receipt.wake.state, "running",
            "worker restart recovery must not call a handed-over process lost"
        );
        assert!(rig.state.owned.agent_wakes.processes.contains(&id));
        // What the worker does when its home cancels the wake.
        let revision = receipt.revision;
        receipt.wake.state = "cancelling".into();
        store
            .agent_lifecycle(Operation::LeasedProcess(
                crate::durable_state::agent_lifecycle::LeasedProcessMutation::Update {
                    id: id.clone(),
                    expected_revision: revision,
                    receipt,
                },
            ))
            .unwrap();
        rig.state.owned.agent_wakes.processes.terminate(&id);
        let deadline = Instant::now() + Duration::from_secs(20);
        while store
            .leased_process_receipt(&id)
            .unwrap()
            .unwrap()
            .wake
            .state
            != "cancelled"
        {
            assert!(Instant::now() < deadline, "the leased process never ended");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone(pid), "the cancelled process is still running");
    }

    #[tokio::test]
    async fn a_wake_being_cancelled_when_the_kernel_reloads_still_ends_its_process() {
        crate::test_support::isolated_env_test!();
        // Ignores SIGTERM, so only the escalation to SIGKILL ends it.
        let rig = watching(&["sh", "-c", "trap '' TERM; sleep 60"], WatchRig::default()).await;
        // The owner cancels the agent's task, as a person does.
        let store = &rig.state.owned.durable_state_store;
        let task = store
            .agent_tasks(Some(&rig.room), Some(&rig.agent))
            .unwrap()
            .remove(0);
        let owner = task.owner_user_id.clone();
        let Outcome::Task(cancelled) = store
            .agent_lifecycle(Operation::CancelTask {
                task: task.task_id,
                owner,
                revision: task.revision,
            })
            .unwrap()
        else {
            panic!("the task was not cancelled")
        };
        rig.state
            .cancel_agent_task_resources(&cancelled)
            .await
            .unwrap();
        assert_eq!(
            wake_state(&rig),
            "cancelling",
            "the process is still being stopped when the kernel reloads"
        );
        let (rig, handed_over, _resources) = restarted(rig).await;
        let (pid, birth) = (handed_over[0].pid(), handed_over[0].birth);
        let _kill = KillOnDrop(pid, birth);
        rig.state
            .adopt_watched_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        wake_ends(&rig, "cancelled").await;
        assert!(gone(pid), "the cancelled process is still running");
    }

    #[tokio::test]
    async fn a_match_seen_as_the_kernel_reloads_is_delivered_once() {
        crate::test_support::isolated_env_test!();
        let rig = WatchRig {
            match_text: Some("ready"),
            ..Default::default()
        };
        // No newline: the line is flushed as supervision stops.
        let (rig, handed_over, _resources) =
            restarted(watching(&["sh", "-c", "printf ready; sleep 60"], rig).await).await;
        let (pid, birth) = (handed_over[0].pid(), handed_over[0].birth);
        let _kill = KillOnDrop(pid, birth);
        rig.state
            .adopt_watched_processes(handed_over, crate::session::unix_epoch_ms())
            .unwrap();
        let wake = wake_matched(&rig).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(wake.fire_count, 1, "{wake:?}");
    }

    async fn wake_matched(rig: &YieldedWatches) -> AgentWake {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let wake = rig
                .state
                .owned
                .durable_state_store
                .agent_wakes(Some(&rig.room), Some(&rig.agent))
                .unwrap()
                .into_iter()
                .find(|wake| wake.id == rig.wakes[0].id)
                .unwrap();
            if wake.matched_at_ms.is_some() {
                return wake;
            }
            assert!(
                Instant::now() < deadline,
                "the match was never delivered: {wake:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
