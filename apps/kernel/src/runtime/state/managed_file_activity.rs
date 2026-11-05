use super::*;

/// A blocking file task retains this guard even if its requesting future is
/// cancelled. A stop must wait until the actual disk operation has finished.
pub(crate) struct ManagedFileOperation {
    runtime: KernelRuntimeState,
}

impl KernelRuntimeState {
    pub(crate) fn begin_file_operation(&self) -> Result<ManagedFileOperation, DaemonError> {
        let admission = self
            .owned
            .managed_kernel_quiescence
            .as_ref()
            .map(|gate| gate.admission_guard())
            .transpose()?;
        let mutation = self.owned.begin_managed_activity_mutation();
        self.owned
            .active_file_operations
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        mutation.record();
        drop(admission);
        self.owned.runtime_projection_changes.record_change();
        Ok(ManagedFileOperation {
            runtime: self.clone(),
        })
    }
}

impl Drop for ManagedFileOperation {
    fn drop(&mut self) {
        let mutation = self.runtime.owned.begin_managed_activity_mutation();
        self.runtime
            .owned
            .active_file_operations
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        mutation.record();
        self.runtime
            .owned
            .runtime_projection_changes
            .record_change();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::runtime::router::CommandRouter;

    #[tokio::test]
    async fn file_work_remains_busy_until_the_blocking_task_finishes() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("file-kernel")
            .unwrap();
        let operation = runtime.begin_file_operation().unwrap();
        assert_eq!(runtime.managed_working_agent_count(), 1);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let _operation = operation;
            entered_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
        });
        entered_rx.await.unwrap();
        worker.abort();
        assert_eq!(runtime.managed_working_agent_count(), 1);
        finish_tx.send(()).unwrap();
        worker.await.unwrap();
        assert_eq!(runtime.managed_working_agent_count(), 0);
    }

    #[tokio::test]
    async fn admitted_file_work_prevents_stop_and_reserved_stop_refuses_file_work() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
        ));
        let mut runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime.owned.managed_kernel_quiescence = Some(
            ManagedKernelQuiescenceGate::restore(
                runtime.owned.durable_state_store.clone(),
                "file-kernel".into(),
                runtime.owned.managed_activity_mutation_lock.clone(),
            )
            .unwrap(),
        );
        runtime
            .ensure_managed_activity_tracking("file-kernel")
            .unwrap();
        let (_, idle, transition) = runtime
            .managed_activity_report_snapshot_with_transition()
            .unwrap();
        runtime.confirm_managed_activity_report(7, transition, idle);
        let mut challenge =
            super::super::managed_kernel_quiescence_state::ManagedKernelQuiescenceChallenge {
                challenge_id: "file-stop".into(),
                account_id: "account".into(),
                environment_id: "environment".into(),
                machine_id: "machine".into(),
                kernel_id: "file-kernel".into(),
                desired_revision: 1,
                idle_sequence: 7,
                idle_deadline_at: "2026-11-05T00:00:00.000Z".into(),
                stop_operation_id: "stop".into(),
                nonce: "nonce".into(),
            };
        let operation = runtime.begin_file_operation().unwrap();
        assert!(!runtime
            .reserve_managed_kernel_for_stop(challenge.clone())
            .unwrap());
        drop(operation);
        let (_, idle, transition) = runtime
            .managed_activity_report_snapshot_with_transition()
            .unwrap();
        runtime.confirm_managed_activity_report(8, transition, idle);
        challenge.idle_sequence = 8;
        assert!(runtime.reserve_managed_kernel_for_stop(challenge).unwrap());
        assert!(runtime.begin_file_operation().is_err());
        assert_eq!(runtime.managed_working_agent_count(), 0);
    }

    #[tokio::test]
    async fn file_admission_cannot_overtake_a_stop_reservation() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
        ));
        let mut runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let gate = ManagedKernelQuiescenceGate::restore(
            runtime.owned.durable_state_store.clone(),
            "file-race".into(),
            runtime.owned.managed_activity_mutation_lock.clone(),
        )
        .unwrap();
        runtime.owned.managed_kernel_quiescence = Some(gate.clone());
        runtime
            .ensure_managed_activity_tracking("file-race")
            .unwrap();
        let (_, idle, transition) = runtime
            .managed_activity_report_snapshot_with_transition()
            .unwrap();
        runtime.confirm_managed_activity_report(1, transition, idle);
        let challenge =
            super::super::managed_kernel_quiescence_state::ManagedKernelQuiescenceChallenge {
                challenge_id: "file-race-stop".into(),
                account_id: "account".into(),
                environment_id: "environment".into(),
                machine_id: "machine".into(),
                kernel_id: "file-race".into(),
                desired_revision: 1,
                idle_sequence: 1,
                idle_deadline_at: "2026-11-05T00:00:00.000Z".into(),
                stop_operation_id: "stop".into(),
                nonce: "nonce".into(),
            };
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let reserving_runtime = runtime.clone();
        let reserving = std::thread::spawn(move || {
            gate.reserve_if_current(challenge, || {
                entered_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                reserving_runtime
                    .owned
                    .current_managed_activity_observation_unlocked()
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (admitted_tx, admitted_rx) = std::sync::mpsc::channel();
        let racing = std::thread::spawn(move || {
            admitted_tx
                .send(runtime.begin_file_operation().is_ok())
                .unwrap()
        });
        let overtook = admitted_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .ok();
        release_tx.send(()).unwrap();
        assert!(reserving.join().unwrap().unwrap());
        racing.join().unwrap();
        assert!(
            overtook.is_none(),
            "file admission overtook the idle recheck"
        );
        assert!(!admitted_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap());
    }
}
