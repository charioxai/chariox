use super::*;

impl ProviderRunActorMailbox {
    pub(crate) fn spawn_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        run: RuntimeProviderRun,
        envelope: PromptEnvelope,
    ) -> Result<(), DaemonError> {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        match sender.try_send(ProviderRunActorCommand::Submit {
            session_id,
            provider_run_id: provider_run_id.clone(),
            agent_id,
            prompt_id,
            run,
            envelope,
        }) {
            Ok(()) => {
                self.operation_lanes.record_command_enqueued();
                Ok(())
            }
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                self.clear_structured_prompt_io_in_flight(&provider_run_id);
                let error_message = error.to_string();
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt submit command enqueue failed",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error_message,
                    }),
                );
                Err(provider_actor_enqueue_error(
                    "enqueue structured prompt submit",
                    &provider_run_id,
                    error_message,
                ))
            }
        }
    }

    pub(crate) fn run_utility(
        &self,
        provider_run_id: String,
        run: RuntimeProviderRun,
        envelope: PromptEnvelope,
        timeout: std::time::Duration,
    ) -> Result<String, DaemonError> {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        match sender.try_send(ProviderRunActorCommand::Utility {
            provider_run_id: provider_run_id.clone(),
            run,
            envelope,
            timeout,
            response: response_tx,
        }) {
            Ok(()) => self.operation_lanes.record_command_enqueued(),
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                self.clear_structured_prompt_io_in_flight(&provider_run_id);
                return Err(provider_actor_enqueue_error(
                    "enqueue structured utility prompt",
                    &provider_run_id,
                    error.to_string(),
                ));
            }
        }
        response_rx
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run structured utility prompt",
                message: format!("provider run actor utility response failed: {error}"),
            })?
    }

    pub(crate) fn spawn_abort(
        &self,
        session_id: String,
        provider_run_id: String,
        run: RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        match sender.try_send(ProviderRunActorCommand::Abort {
            session_id,
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            Ok(()) => {
                self.operation_lanes.record_command_enqueued();
                Ok(())
            }
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                self.clear_structured_prompt_io_in_flight(&provider_run_id);
                let error_message = error.to_string();
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt abort command enqueue failed",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error_message,
                    }),
                );
                Err(provider_actor_enqueue_error(
                    "enqueue structured prompt abort",
                    &provider_run_id,
                    error_message,
                ))
            }
        }
    }

    pub(crate) fn spawn_terminate(&self, provider_run_id: String, run: RuntimeProviderRun) {
        self.operation_lanes.forget(&provider_run_id);
        let sender = {
            let mut workers = self
                .workers
                .lock()
                .expect("provider run actor worker map poisoned");
            workers
                .remove(&provider_run_id)
                .unwrap_or_else(|| self.worker_deps().spawn(provider_run_id.clone()))
        };
        let (completion_tx, completion_rx) = mpsc::channel();
        match sender.try_send(ProviderRunActorCommand::Terminate {
            provider_run_id: provider_run_id.clone(),
            run,
            completion: completion_tx,
        }) {
            Ok(()) => {
                self.operation_lanes.record_command_enqueued();
                if let Err(error) = completion_rx.recv_timeout(Duration::from_secs(15)) {
                    crate::logging::error_with_fields(
                        "daemon.provider_run_actor",
                        "provider run actor terminate command did not complete before cleanup",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                self.clear_structured_prompt_io_in_flight(&provider_run_id);
                self.clear_runtime(&provider_run_id);
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run actor terminate command enqueue failed",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    pub(crate) fn spawn_selection_sync(
        &self,
        provider_run_id: String,
        run: RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let sender = self.worker_for_run(&provider_run_id);
        match sender.try_send(ProviderRunActorCommand::SyncSelection {
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            Ok(()) => {
                self.operation_lanes.record_command_enqueued();
                Ok(())
            }
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                let error_message = error.to_string();
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run selection sync command enqueue failed",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error_message,
                    }),
                );
                Err(provider_actor_enqueue_error(
                    "enqueue provider run selection sync",
                    &provider_run_id,
                    error_message,
                ))
            }
        }
    }

    pub(crate) fn spawn_output_poll(
        &self,
        provider_run_id: String,
        run: RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        if !self.mark_structured_output_poll_in_flight(provider_run_id.clone()) {
            return Ok(false);
        }
        let sender = self.worker_for_run(&provider_run_id);
        match sender.try_send(ProviderRunActorCommand::PollOutput {
            provider_run_id: provider_run_id.clone(),
            run,
            output_poll_delay: self.output_poll_delay_for_run(&provider_run_id),
        }) {
            Ok(()) => {
                self.operation_lanes.record_command_enqueued();
                Ok(true)
            }
            Err(error) => {
                self.operation_lanes.record_enqueue_rejection();
                self.clear_structured_output_poll_in_flight(&provider_run_id);
                let error_message = error.to_string();
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run output poll command enqueue failed",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "error": error_message,
                    }),
                );
                Err(provider_actor_enqueue_error(
                    "enqueue provider run output poll",
                    &provider_run_id,
                    error_message,
                ))
            }
        }
    }

    fn output_poll_delay_for_run(&self, run_id: &str) -> Duration {
        self.output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned")
            .get(run_id)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn stop_run(&self, provider_run_id: &str) {
        self.operation_lanes.forget(provider_run_id);
        self.output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned")
            .remove(provider_run_id);
        let sender = {
            let mut workers = self
                .workers
                .lock()
                .expect("provider run actor worker map poisoned");
            workers.remove(provider_run_id)
        };
        if let Some(sender) = sender {
            match sender.try_send(ProviderRunActorCommand::Stop) {
                Ok(()) => self.operation_lanes.record_command_enqueued(),
                Err(_) => self.operation_lanes.record_enqueue_rejection(),
            }
        }
    }

    pub(crate) fn drain_finished_submits(&self) -> Vec<FinishedProviderPromptSubmitJob> {
        drain_finished_submits(&self.finished_submits)
    }

    pub(crate) fn drain_finished_aborts(&self) -> Vec<FinishedProviderPromptAbortJob> {
        drain_finished_aborts(&self.finished_aborts)
    }

    pub(crate) fn drain_finished_selection_syncs(
        &self,
    ) -> Vec<FinishedProviderRunSelectionSyncJob> {
        drain_finished_selection_syncs(&self.finished_selection_syncs)
    }

    pub(crate) fn drain_finished_output_polls(&self) -> Vec<FinishedProviderOutputPollJob> {
        drain_finished_output_polls(&self.finished_output_polls)
    }

    #[cfg(test)]
    pub(crate) fn push_finished_output_poll_for_test(
        &self,
        finished: FinishedProviderOutputPollJob,
    ) {
        let provider_run_id = finished.provider_run_id.clone();
        push_finished_output_poll(&self.finished_output_polls, finished);
        self.completion_signal.record_completion(&provider_run_id);
    }

    pub(super) fn worker_for_run(
        &self,
        provider_run_id: &str,
    ) -> tokio_mpsc::Sender<ProviderRunActorCommand> {
        let mut workers = self
            .workers
            .lock()
            .expect("provider run actor worker map poisoned");
        workers
            .entry(provider_run_id.to_string())
            .or_insert_with(|| self.worker_deps().spawn(provider_run_id.to_string()))
            .clone()
    }

    fn worker_deps(&self) -> ProviderRunWorkerDeps {
        ProviderRunWorkerDeps {
            native_interaction_bridge: self.native_interaction_bridge.clone(),
            runtime_registry: self.runtime_registry.clone(),
            in_flight: self.in_flight.clone(),
            finished_submits: Arc::clone(&self.finished_submits),
            finished_aborts: Arc::clone(&self.finished_aborts),
            finished_selection_syncs: Arc::clone(&self.finished_selection_syncs),
            finished_output_polls: Arc::clone(&self.finished_output_polls),
            completion_signal: self.completion_signal.clone(),
            output_poll_delays: Arc::clone(&self.output_poll_delays),
            blocking_executor_permits: Arc::clone(&self.blocking_executor_permits),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::error::DaemonError;
    use crate::provider::RuntimeProviderRun;

    use super::*;

    #[tokio::test]
    async fn stop_run_removes_worker_and_lane_registration() {
        let mailbox = ProviderRunActorMailbox::default();
        let _sender = mailbox.worker_for_run("run-1");
        let _permit = mailbox.operation_lanes.acquire("run-1").await;
        mailbox.mark_structured_prompt_io_in_flight("run-1".to_string());
        assert!(mailbox.mark_structured_output_poll_in_flight("run-1".to_string()));
        assert_eq!(
            mailbox
                .workers
                .lock()
                .expect("worker map should not be poisoned")
                .len(),
            1
        );
        assert_eq!(
            mailbox
                .operation_lanes
                .lanes
                .lock()
                .expect("lane map should not be poisoned")
                .len(),
            1
        );
        assert!(mailbox.structured_prompt_io_in_flight("run-1"));

        mailbox.clear_runtime("run-1");
        mailbox.stop_run("run-1");

        assert_eq!(
            mailbox
                .workers
                .lock()
                .expect("worker map should not be poisoned")
                .len(),
            0
        );
        assert_eq!(
            mailbox
                .operation_lanes
                .lanes
                .lock()
                .expect("lane map should not be poisoned")
                .len(),
            0
        );
        assert!(!mailbox.structured_prompt_io_in_flight("run-1"));
        assert!(!mailbox.structured_output_poll_in_flight("run-1"));
        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueued_commands,
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminate_completes_while_the_calling_runtime_thread_is_blocked() {
        let mailbox = ProviderRunActorMailbox::default();
        let run = runtime_run("run-terminate");

        mailbox.spawn_terminate("run-terminate".to_string(), run);

        assert!(!mailbox
            .workers
            .lock()
            .expect("worker map should not be poisoned")
            .contains_key("run-terminate"));
    }

    #[tokio::test]
    async fn finished_output_poll_wakes_completion_waiters() {
        let mailbox = ProviderRunActorMailbox::default();
        let signal = mailbox.completion_signal();
        let sequence = signal.sequence();

        mailbox.push_finished_output_poll_for_test(FinishedProviderOutputPollJob {
            provider_run_id: "run-1".to_string(),
            result: Ok(None),
        });

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            signal.wait_for_change_after(sequence),
        )
        .await
        .expect("finished structured output poll should wake completion waiters");
        assert_eq!(
            signal.take_ready_provider_run_ids(),
            ["run-1".to_string()].into_iter().collect()
        );
        assert!(signal.take_ready_provider_run_ids().is_empty());
        assert_eq!(mailbox.drain_finished_output_polls().len(), 1);
    }

    #[test]
    fn provider_run_actor_health_counts_enqueue_rejections() {
        let lanes = ProviderRunOperationLanes::default();
        lanes.record_command_enqueued();
        lanes.record_enqueue_rejection();

        let snapshot = lanes.health_snapshot();

        assert_eq!(snapshot.enqueued_commands, 1);
        assert_eq!(snapshot.enqueue_rejections, 1);
    }

    fn mailbox_with_full_run_queue(run_id: &str) -> ProviderRunActorMailbox {
        let mailbox = ProviderRunActorMailbox::default();
        let (sender, _receiver) = tokio_mpsc::channel(1);
        sender
            .try_send(ProviderRunActorCommand::Stop)
            .expect("test provider run queue should accept first command");
        mailbox
            .workers
            .lock()
            .expect("worker map should not be poisoned")
            .insert(run_id.to_string(), sender);
        mailbox
    }

    fn runtime_run(run_id: &str) -> RuntimeProviderRun {
        RuntimeProviderRun::from_control_capability_inference(
            run_id,
            "session-1".to_string(),
            Some("agent-1".to_string()),
            "codex".to_string(),
        )
    }

    fn assert_local_transport_operation(error: DaemonError, expected_operation: &'static str) {
        match error {
            DaemonError::LocalTransport { operation, .. } => {
                assert_eq!(operation, expected_operation);
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[test]
    fn structured_submit_enqueue_failure_is_reported_and_clears_in_flight_state() {
        let mailbox = mailbox_with_full_run_queue("run-1");

        let error = mailbox
            .spawn_submit(
                "session-1".to_string(),
                "run-1".to_string(),
                "agent-1".to_string(),
                "prompt-1".to_string(),
                runtime_run("run-1"),
                crate::prompt_assembly::PromptEnvelope::new(
                    "hello",
                    "",
                    Vec::new(),
                    crate::prompt_assembly::PromptManifest::default(),
                ),
            )
            .expect_err("full provider actor queue should reject submit");

        assert!(!mailbox.structured_prompt_io_in_flight("run-1"));
        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueue_rejections,
            1
        );
        assert_local_transport_operation(error, "enqueue structured prompt submit");
    }

    #[test]
    fn structured_abort_enqueue_failure_is_reported_and_clears_in_flight_state() {
        let mailbox = mailbox_with_full_run_queue("run-1");

        let error = mailbox
            .spawn_abort(
                "session-1".to_string(),
                "run-1".to_string(),
                runtime_run("run-1"),
            )
            .expect_err("full provider actor queue should reject abort");

        assert!(!mailbox.structured_prompt_io_in_flight("run-1"));
        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueue_rejections,
            1
        );
        assert_local_transport_operation(error, "enqueue structured prompt abort");
    }

    #[test]
    fn structured_output_poll_enqueue_failure_is_reported_and_clears_in_flight_state() {
        let mailbox = mailbox_with_full_run_queue("run-1");

        let error = mailbox
            .spawn_output_poll("run-1".to_string(), runtime_run("run-1"))
            .expect_err("full provider actor queue should reject output poll");

        assert!(!mailbox.structured_output_poll_in_flight("run-1"));
        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueue_rejections,
            1
        );
        assert_local_transport_operation(error, "enqueue provider run output poll");
    }

    #[test]
    fn selection_sync_enqueue_failure_is_reported() {
        let mailbox = mailbox_with_full_run_queue("run-1");

        let error = mailbox
            .spawn_selection_sync("run-1".to_string(), runtime_run("run-1"))
            .expect_err("full provider actor queue should reject selection sync");

        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueue_rejections,
            1
        );
        assert_local_transport_operation(error, "enqueue provider run selection sync");
    }
}
