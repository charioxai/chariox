use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

mod finished_jobs;
mod native_interaction;
mod operation_lanes;
mod runtime_slots;
use finished_jobs::{
    push_finished_abort, push_finished_output_poll, push_finished_selection_sync,
    push_finished_submit,
};
pub(crate) use finished_jobs::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    FinishedProviderRunSelectionSyncJob,
};
pub(crate) use native_interaction::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionBridgeStore,
    ProviderNativeInteractionResolution,
};
pub(crate) use operation_lanes::ProviderRunOperationLanes;
#[cfg(test)]
use runtime_slots::runtime_should_restore;
use runtime_slots::{
    clear_runtime_state, opencode_slot, restore_claude_runtime_if_live,
    restore_codex_runtime_if_live, restore_opencode_runtime_if_live,
    runtime_slot_missing_or_empty_claude, runtime_slot_missing_or_empty_codex,
    runtime_slot_missing_or_empty_opencode, take_claude_runtime, take_codex_runtime,
    take_opencode_runtime, ClaudeRuntimeSlot, CodexRuntimeSlot, OpenCodeRuntimeSlot,
};

use crate::error::DaemonError;
use crate::session::PromptAttachment;

use super::{
    claude_runtime::{abort_claude_turn, drain_claude_events, submit_claude_prompt},
    codex_runtime::{abort_codex_turn, drain_codex_events, submit_codex_prompt},
    opencode_binding::{
        abort_opencode_session, submit_opencode_prompt, sync_opencode_run_selection_for_session,
        OpenCodeRunSelection,
    },
    opencode_runtime::{drain_opencode_events, OpenCodeRuntimeState},
    ClaudeRuntimeState, CodexRuntimeState, ProviderAssistantCompletion, ProviderPromptChunk,
    ProviderPromptSignalBatch, RuntimeProviderRun,
};

const PROVIDER_RUN_COMMAND_QUEUE_LIMIT: usize = 64;

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    operation_lanes: ProviderRunOperationLanes,
    native_interaction_bridge: ProviderNativeInteractionBridgeStore,
    workers: Arc<Mutex<BTreeMap<String, mpsc::SyncSender<ProviderRunActorCommand>>>>,
    claude_runs: Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: Arc<Mutex<BTreeSet<String>>>,
    structured_prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
    structured_output_polls: Arc<Mutex<BTreeSet<String>>>,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
}

enum ProviderRunActorCommand {
    Submit {
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        prompt: String,
        attachments: Vec<PromptAttachment>,
    },
    Abort {
        session_id: String,
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    Terminate {
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    SyncSelection {
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    PollOutput {
        provider_run_id: String,
        run: RuntimeProviderRun,
        output_poll_delay: Duration,
    },
    Stop,
}

impl ProviderRunActorMailbox {
    pub(crate) fn operation_lanes(&self) -> ProviderRunOperationLanes {
        self.operation_lanes.clone()
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.native_interaction_bridge.set(bridge);
    }

    pub(crate) fn insert_claude_runtime(&self, run_id: String, state: ClaudeRuntimeState) {
        self.cleared_runs
            .lock()
            .expect("cleared provider run set poisoned")
            .remove(&run_id);
        self.claude_runs
            .lock()
            .expect("claude runtime map poisoned")
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(crate) fn insert_codex_runtime(&self, run_id: String, state: CodexRuntimeState) {
        self.cleared_runs
            .lock()
            .expect("cleared provider run set poisoned")
            .remove(&run_id);
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(crate) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        self.cleared_runs
            .lock()
            .expect("cleared provider run set poisoned")
            .remove(&run_id);
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .insert(run_id, Arc::new(Mutex::new(Some(state))));
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, run_id: &str) -> bool {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .contains(run_id)
    }

    pub(crate) fn structured_runtime_state_bound(&self, run_id: &str) -> bool {
        if self
            .claude_runs
            .lock()
            .expect("claude runtime map poisoned")
            .get(run_id)
            .is_some_and(|slot| slot.lock().expect("claude runtime slot poisoned").is_some())
        {
            return true;
        }
        if self
            .codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .get(run_id)
            .is_some_and(|slot| slot.lock().expect("codex runtime slot poisoned").is_some())
        {
            return true;
        }
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .get(run_id)
            .is_some_and(|slot| {
                slot.lock()
                    .expect("opencode runtime slot poisoned")
                    .is_some()
            })
    }

    pub(crate) fn mark_structured_prompt_io_in_flight(&self, run_id: String) {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .insert(run_id);
    }

    pub(crate) fn clear_structured_prompt_io_in_flight(&self, run_id: &str) {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .remove(run_id);
    }

    pub(crate) fn clear_runtime(&self, run_id: &str) {
        self.cleared_runs
            .lock()
            .expect("cleared provider run set poisoned")
            .insert(run_id.to_string());
        self.output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned")
            .remove(run_id);
        self.clear_structured_prompt_io_in_flight(run_id);
        self.clear_structured_output_poll_in_flight(run_id);
        clear_runtime_state(
            &self.claude_runs,
            &self.codex_runs,
            &self.opencode_runs,
            run_id,
            true,
        );
    }

    #[doc(hidden)]
    pub(crate) fn set_output_poll_delay_for_tests(&self, run_id: &str, delay: Duration) {
        let mut delays = self
            .output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned");
        if delay.is_zero() {
            delays.remove(run_id);
        } else {
            delays.insert(run_id.to_string(), delay);
        }
    }

    fn mark_structured_output_poll_in_flight(&self, run_id: String) -> bool {
        self.structured_output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .insert(run_id)
    }

    fn clear_structured_output_poll_in_flight(&self, run_id: &str) {
        self.structured_output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .remove(run_id);
    }

    pub(crate) fn spawn_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        prompt: String,
        attachments: Vec<PromptAttachment>,
    ) -> Result<(), DaemonError> {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        match sender.try_send(ProviderRunActorCommand::Submit {
            session_id,
            provider_run_id: provider_run_id.clone(),
            agent_id,
            run,
            prompt,
            attachments,
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
            workers.remove(&provider_run_id).unwrap_or_else(|| {
                Self::spawn_worker(
                    provider_run_id.clone(),
                    self.native_interaction_bridge.clone(),
                    Arc::clone(&self.claude_runs),
                    Arc::clone(&self.codex_runs),
                    Arc::clone(&self.opencode_runs),
                    Arc::clone(&self.cleared_runs),
                    Arc::clone(&self.structured_prompt_submissions),
                    Arc::clone(&self.structured_output_polls),
                    Arc::clone(&self.finished_submits),
                    Arc::clone(&self.finished_aborts),
                    Arc::clone(&self.finished_selection_syncs),
                    Arc::clone(&self.finished_output_polls),
                    Arc::clone(&self.output_poll_delays),
                )
            })
        };
        match sender.try_send(ProviderRunActorCommand::Terminate {
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            Ok(()) => self.operation_lanes.record_command_enqueued(),
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
        match self.finished_submits.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt submit completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn drain_finished_aborts(&self) -> Vec<FinishedProviderPromptAbortJob> {
        match self.finished_aborts.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt abort completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn drain_finished_selection_syncs(
        &self,
    ) -> Vec<FinishedProviderRunSelectionSyncJob> {
        match self.finished_selection_syncs.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run selection sync completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn drain_finished_output_polls(&self) -> Vec<FinishedProviderOutputPollJob> {
        match self.finished_output_polls.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run output poll completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn push_finished_output_poll_for_test(
        &self,
        finished: FinishedProviderOutputPollJob,
    ) {
        push_finished_output_poll(&self.finished_output_polls, finished);
    }

    fn worker_for_run(&self, provider_run_id: &str) -> mpsc::SyncSender<ProviderRunActorCommand> {
        let mut workers = self
            .workers
            .lock()
            .expect("provider run actor worker map poisoned");
        workers
            .entry(provider_run_id.to_string())
            .or_insert_with(|| {
                Self::spawn_worker(
                    provider_run_id.to_string(),
                    self.native_interaction_bridge.clone(),
                    Arc::clone(&self.claude_runs),
                    Arc::clone(&self.codex_runs),
                    Arc::clone(&self.opencode_runs),
                    Arc::clone(&self.cleared_runs),
                    Arc::clone(&self.structured_prompt_submissions),
                    Arc::clone(&self.structured_output_polls),
                    Arc::clone(&self.finished_submits),
                    Arc::clone(&self.finished_aborts),
                    Arc::clone(&self.finished_selection_syncs),
                    Arc::clone(&self.finished_output_polls),
                    Arc::clone(&self.output_poll_delays),
                )
            })
            .clone()
    }

    fn spawn_worker(
        provider_run_id: String,
        native_interaction_bridge: ProviderNativeInteractionBridgeStore,
        claude_runs: Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
        codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
        opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
        cleared_runs: Arc<Mutex<BTreeSet<String>>>,
        structured_prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
        structured_output_polls: Arc<Mutex<BTreeSet<String>>>,
        finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
        finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
        finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
        finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
        output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
    ) -> mpsc::SyncSender<ProviderRunActorCommand> {
        let (tx, rx) = mpsc::sync_channel(PROVIDER_RUN_COMMAND_QUEUE_LIMIT);
        thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    ProviderRunActorCommand::Submit {
                        session_id,
                        provider_run_id,
                        agent_id,
                        run,
                        prompt,
                        attachments,
                    } => {
                        let result = execute_submit_command(
                            &claude_runs,
                            &codex_runs,
                            &opencode_runs,
                            &cleared_runs,
                            run,
                            prompt,
                            attachments,
                        );
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderPromptSubmitJob {
                            session_id,
                            provider_run_id,
                            agent_id,
                            result,
                        };
                        push_finished_submit(&finished_submits, finished);
                    }
                    ProviderRunActorCommand::Abort {
                        session_id,
                        provider_run_id,
                        run,
                    } => {
                        let result = execute_abort_command(
                            &claude_runs,
                            &codex_runs,
                            &opencode_runs,
                            &cleared_runs,
                            run,
                        );
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderPromptAbortJob {
                            session_id,
                            provider_run_id,
                            result,
                        };
                        push_finished_abort(&finished_aborts, finished);
                    }
                    ProviderRunActorCommand::Terminate {
                        provider_run_id,
                        run,
                    } => {
                        if let Err(error) = execute_terminate_command(
                            &claude_runs,
                            &codex_runs,
                            &opencode_runs,
                            &cleared_runs,
                            run,
                        ) {
                            crate::logging::error_with_fields(
                                "daemon.provider_run_actor",
                                "structured provider termination abort failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        output_poll_delays
                            .lock()
                            .expect("provider output poll delay map poisoned")
                            .remove(&provider_run_id);
                        clear_runtime_state(
                            &claude_runs,
                            &codex_runs,
                            &opencode_runs,
                            &provider_run_id,
                            true,
                        );
                        break;
                    }
                    ProviderRunActorCommand::SyncSelection {
                        provider_run_id,
                        run,
                    } => {
                        let result =
                            execute_selection_sync_command(&opencode_runs, &provider_run_id, &run);
                        let finished = FinishedProviderRunSelectionSyncJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_selection_sync(&finished_selection_syncs, finished);
                    }
                    ProviderRunActorCommand::PollOutput {
                        provider_run_id,
                        run,
                        output_poll_delay,
                    } => {
                        let result = execute_output_poll_command(
                            &native_interaction_bridge,
                            &claude_runs,
                            &codex_runs,
                            &opencode_runs,
                            &cleared_runs,
                            &run,
                            output_poll_delay,
                        );
                        clear_structured_output_poll_in_flight(
                            &structured_output_polls,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderOutputPollJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_output_poll(&finished_output_polls, finished);
                    }
                    ProviderRunActorCommand::Stop => break,
                }
            }
            crate::logging::info_with_fields(
                "daemon.provider_run_actor",
                "provider run actor worker stopped",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                }),
            );
        });
        tx
    }
}

#[cfg(test)]
mod tests {
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
        assert!(mailbox
            .structured_output_polls
            .lock()
            .expect("structured output poll set should not be poisoned")
            .is_empty());
        assert_eq!(
            mailbox.operation_lanes.health_snapshot().enqueued_commands,
            1
        );
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
        let (sender, _receiver) = std::sync::mpsc::sync_channel(0);
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
                runtime_run("run-1"),
                "hello".to_string(),
                Vec::new(),
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

        assert!(mailbox
            .structured_output_polls
            .lock()
            .expect("structured output poll set should not be poisoned")
            .is_empty());
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

    #[test]
    fn runtime_tombstone_rejects_stale_state_restore_after_cleanup() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let slot = Arc::new(Mutex::new(None));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&slot));

        assert!(runtime_should_restore(&cleared_runs, &runs, "run-1", &slot));

        cleared_runs
            .lock()
            .expect("cleared set should not be poisoned")
            .insert("run-1".to_string());
        runs.lock()
            .expect("runtime map should not be poisoned")
            .remove("run-1");

        assert!(!runtime_should_restore(
            &cleared_runs,
            &runs,
            "run-1",
            &slot
        ));
    }

    #[test]
    fn runtime_restore_drops_taken_state_after_cleanup_tombstone() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let slot = Arc::new(Mutex::new(Some(7)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&slot));

        let taken_state = slot
            .lock()
            .expect("runtime slot should not be poisoned")
            .take()
            .expect("runtime state should be present");

        cleared_runs
            .lock()
            .expect("cleared set should not be poisoned")
            .insert("run-1".to_string());
        runs.lock()
            .expect("runtime map should not be poisoned")
            .remove("run-1");

        if runtime_should_restore(&cleared_runs, &runs, "run-1", &slot) {
            *slot.lock().expect("runtime slot should not be poisoned") = Some(taken_state);
        }

        assert!(!runs
            .lock()
            .expect("runtime map should not be poisoned")
            .contains_key("run-1"));
        assert!(slot
            .lock()
            .expect("runtime slot should not be poisoned")
            .is_none());
    }

    #[test]
    fn runtime_restore_rejects_old_slot_after_same_run_replacement() {
        let cleared_runs = Arc::new(Mutex::new(BTreeSet::new()));
        let runs: Arc<Mutex<BTreeMap<String, Arc<Mutex<Option<i32>>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let old_slot = Arc::new(Mutex::new(Some(7)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&old_slot));

        let taken_state = old_slot
            .lock()
            .expect("old runtime slot should not be poisoned")
            .take()
            .expect("old runtime state should be present");
        let replacement_slot = Arc::new(Mutex::new(Some(42)));
        runs.lock()
            .expect("runtime map should not be poisoned")
            .insert("run-1".to_string(), Arc::clone(&replacement_slot));

        if runtime_should_restore(&cleared_runs, &runs, "run-1", &old_slot) {
            *old_slot
                .lock()
                .expect("old runtime slot should not be poisoned") = Some(taken_state);
        }

        assert!(old_slot
            .lock()
            .expect("old runtime slot should not be poisoned")
            .is_none());
        assert_eq!(
            *replacement_slot
                .lock()
                .expect("replacement runtime slot should not be poisoned"),
            Some(42)
        );
    }
}

fn provider_actor_enqueue_error(
    operation: &'static str,
    provider_run_id: &str,
    error_message: String,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: format!(
            "provider run actor queue rejected command for `{provider_run_id}`: {error_message}"
        ),
    }
}

fn execute_submit_command(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run: RuntimeProviderRun,
    prompt: String,
    attachments: Vec<PromptAttachment>,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(std::time::Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = take_codex_runtime(codex_runs, &run_id)?;
        let result = submit_codex_prompt(&run, &mut state, &prompt, &attachments);
        restore_codex_runtime_if_live(codex_runs, cleared_runs, &run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "claude" {
        if !run.client_interface().is_arroba() {
            return Ok(());
        }
        let (slot, mut state) = take_claude_runtime(claude_runs, &run_id)?;
        let result = submit_claude_prompt(&run, &mut state, &prompt, &attachments);
        restore_claude_runtime_if_live(claude_runs, cleared_runs, &run_id, &slot, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let (slot, mut state) = take_opencode_runtime(opencode_runs, &run_id)?;
    let result = submit_opencode_prompt(&run, &mut state, &prompt, &attachments);
    restore_opencode_runtime_if_live(opencode_runs, cleared_runs, &run_id, &slot, state);
    result
}

fn execute_abort_command(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(std::time::Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = take_codex_runtime(codex_runs, &run_id)?;
        let result = abort_codex_turn(&run_id, &mut state);
        restore_codex_runtime_if_live(codex_runs, cleared_runs, &run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "claude" {
        if !run.client_interface().is_arroba() {
            return Ok(());
        }
        let (slot, mut state) = take_claude_runtime(claude_runs, &run_id)?;
        let result = abort_claude_turn(&run, &mut state);
        restore_claude_runtime_if_live(claude_runs, cleared_runs, &run_id, &slot, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let (slot, state) = take_opencode_runtime(opencode_runs, &run_id)?;
    let result = abort_opencode_session(&run_id, &state);
    restore_opencode_runtime_if_live(opencode_runs, cleared_runs, &run_id, &slot, state);
    result
}

fn execute_terminate_command(
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "codex" && runtime_slot_missing_or_empty_codex(codex_runs, &run_id) {
        return Ok(());
    }
    if run.adapter_key() == "claude" && runtime_slot_missing_or_empty_claude(claude_runs, &run_id) {
        return Ok(());
    }
    if run.adapter_key() == "opencode"
        && runtime_slot_missing_or_empty_opencode(opencode_runs, &run_id)
    {
        return Ok(());
    }
    execute_abort_command(claude_runs, codex_runs, opencode_runs, cleared_runs, run)
}

fn execute_selection_sync_command(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    run_id: &str,
    run: &RuntimeProviderRun,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let slot = opencode_slot(opencode_runs, run_id)?;
    let (base_url, session_id) = {
        let guard = slot.lock().expect("opencode runtime slot poisoned");
        let state = guard
            .as_ref()
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            })?;
        (state.base_url().to_string(), state.session_id().to_string())
    };
    sync_opencode_run_selection_for_session(
        run_id,
        &base_url,
        &session_id,
        run.model(),
        run.variant(),
    )
}

fn execute_output_poll_command(
    native_interaction_bridge: &ProviderNativeInteractionBridgeStore,
    claude_runs: &Arc<Mutex<BTreeMap<String, ClaudeRuntimeSlot>>>,
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeSlot>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeSlot>>>,
    cleared_runs: &Arc<Mutex<BTreeSet<String>>>,
    run: &RuntimeProviderRun,
    output_poll_delay: Duration,
) -> Result<Option<ProviderPromptSignalBatch>, DaemonError> {
    let run_id = run.id();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(Duration::from_millis(750));
        return Ok(None);
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = match take_codex_runtime(codex_runs, run_id) {
            Ok((slot, state)) => (slot, state),
            Err(_) => return Ok(None),
        };
        if !output_poll_delay.is_zero() {
            thread::sleep(output_poll_delay);
        }
        let poll = drain_codex_events(run, &mut state, native_interaction_bridge.read());
        restore_codex_runtime_if_live(codex_runs, cleared_runs, run_id, &slot, state);
        let poll = poll?;
        crate::logging::debug_with_fields(
            "daemon.provider_run_actor",
            "codex output poll result trace",
            serde_json::json!({
                "provider_run_id": run_id,
                "chunks": poll.chunks.len(),
                "completions": poll.completions.len(),
                "prompt_completed": poll.prompt_completed,
                "terminal_failure": poll.terminal_failure,
                "notices": poll.notices.len(),
            }),
        );
        return Ok(Some(ProviderPromptSignalBatch {
            chunks: poll
                .chunks
                .into_iter()
                .map(|chunk| ProviderPromptChunk {
                    kind: chunk.kind,
                    merge_key: chunk.merge_key,
                    bytes: chunk.bytes,
                })
                .collect(),
            completions: poll
                .completions
                .into_iter()
                .map(|completion| ProviderAssistantCompletion {
                    message_id: completion.message_id,
                    completed_at_ms: completion.completed_at_ms,
                })
                .collect(),
            prompt_completed: poll.prompt_completed,
            terminal_failure: poll.terminal_failure,
            notices: poll.notices,
            resolved_model: None,
            resolved_model_source: None,
            resolved_variant: None,
            resolved_usage_tokens_total: poll.resolved_usage.and_then(|usage| usage.total_tokens),
            resolved_usage: poll.resolved_usage,
            resolved_resume_state: None,
        }));
    }
    if run.adapter_key() == "claude" {
        let (slot, mut state) = match take_claude_runtime(claude_runs, run_id) {
            Ok((slot, state)) => (slot, state),
            Err(_) => return Ok(None),
        };
        if !output_poll_delay.is_zero() {
            thread::sleep(output_poll_delay);
        }
        let drain = drain_claude_events(run, &mut state);
        restore_claude_runtime_if_live(claude_runs, cleared_runs, run_id, &slot, state);
        return drain.map(Some);
    }
    if run.adapter_key() != "opencode" {
        return Ok(None);
    }
    let (slot, mut state) = match take_opencode_runtime(opencode_runs, run_id) {
        Ok((slot, state)) => (slot, state),
        Err(_) => return Ok(None),
    };
    if !output_poll_delay.is_zero() {
        thread::sleep(output_poll_delay);
    }
    let drain = drain_opencode_events(run, &mut state, native_interaction_bridge.read());
    restore_opencode_runtime_if_live(opencode_runs, cleared_runs, run_id, &slot, state);
    let drain = drain?;
    Ok(Some(ProviderPromptSignalBatch {
        chunks: drain
            .chunks
            .into_iter()
            .map(|chunk| ProviderPromptChunk {
                kind: chunk.kind,
                merge_key: chunk.merge_key,
                bytes: chunk.bytes,
            })
            .collect(),
        completions: drain
            .completions
            .into_iter()
            .map(|completion| ProviderAssistantCompletion {
                message_id: completion.message_id,
                completed_at_ms: completion.completed_at_ms,
            })
            .collect(),
        prompt_completed: drain.prompt_completed,
        terminal_failure: drain.terminal_failure,
        notices: drain.notices,
        resolved_model: drain.resolved_model,
        resolved_model_source: drain.resolved_model_source,
        resolved_variant: drain.resolved_variant,
        resolved_usage_tokens_total: drain.resolved_usage_tokens_total,
        resolved_usage: None,
        resolved_resume_state: None,
    }))
}

fn clear_structured_prompt_io_in_flight(
    structured_prompt_submissions: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
) {
    structured_prompt_submissions
        .lock()
        .expect("structured prompt submission set poisoned")
        .remove(run_id);
}

fn clear_structured_output_poll_in_flight(
    structured_output_polls: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
) {
    structured_output_polls
        .lock()
        .expect("structured output poll set poisoned")
        .remove(run_id);
}
