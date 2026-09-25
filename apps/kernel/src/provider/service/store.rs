use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptAssemblyMode;
use crate::provider::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    LaunchProviderRequest, ProviderNativeInteractionBridge, ProviderPromptSignalBatch,
    ProviderRegistry, ProviderRunOperationLanes, ProviderRunTokenUsage, RuntimeProviderRun,
};
use crate::session::PromptAttachment;

use super::{
    ProviderProcessService, ProviderRunEndedOutcome, ProviderRunLivenessReconciliation,
    ProviderRunParkedOutcome, ProviderRunResumedOutcome, ProviderRunStartedOutcome,
    ProviderRuntimeBinding, ProviderSessionRunsTerminatedOutcome,
};

#[derive(Clone)]
pub struct ProviderProcessServiceStore {
    inner: Arc<Mutex<ProviderProcessService>>,
}

#[cfg(test)]
mod manifest_revision_tests {
    use super::*;

    fn manifest(available: bool) -> crate::extension::RemoteExtensionManifest {
        crate::extension::RemoteExtensionManifest {
            room_browser_available: available,
            ..Default::default()
        }
    }

    fn fixture() -> ProviderProcessServiceStore {
        let mut service = ProviderProcessService::new();
        service.insert_run_for_test(RuntimeProviderRun::from_control_capability_inference(
            "run",
            "room".to_string(),
            Some("agent".to_string()),
            "codex".to_string(),
        ));
        ProviderProcessServiceStore::new(service)
    }

    #[test]
    fn stale_manifest_compare_update_preserves_newer_snapshot() {
        let store = fixture();
        let before = store.get_run("run").unwrap();
        assert_eq!(before.remote_extension_manifest_revision(), 0);
        let pushed = store
            .update_run_remote_extension_manifest("run", manifest(true))
            .unwrap();
        assert_eq!(pushed.remote_extension_manifest_revision(), 1);
        assert!(store
            .compare_update_run_remote_extension_manifest(
                "run",
                before.remote_extension_manifest_revision(),
                manifest(false),
            )
            .unwrap()
            .is_none());
        assert_eq!(store.get_run("run").unwrap(), pushed);
        let accepted = store
            .compare_update_run_remote_extension_manifest(
                "run",
                pushed.remote_extension_manifest_revision(),
                manifest(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(accepted.remote_extension_manifest_revision(), 2);
        assert!(!accepted.remote_extension_manifest().room_browser_available);
    }

    #[test]
    fn equal_and_aba_manifest_updates_invalidate_old_revisions() {
        let store = fixture();
        store
            .update_run_remote_extension_manifest("run", manifest(false))
            .unwrap();
        store
            .update_run_remote_extension_manifest("run", manifest(true))
            .unwrap();
        let current = store
            .update_run_remote_extension_manifest("run", manifest(false))
            .unwrap();
        assert_eq!(current.remote_extension_manifest_revision(), 3);
        for stale in 0..3 {
            assert!(store
                .compare_update_run_remote_extension_manifest("run", stale, manifest(true),)
                .unwrap()
                .is_none());
        }
        assert_eq!(store.get_run("run").unwrap(), current);
    }

    #[test]
    fn manifest_revision_is_not_serialized_or_accepted_from_serialized_input() {
        let store = fixture();
        let updated = store
            .update_run_remote_extension_manifest("run", manifest(true))
            .unwrap();
        let mut serialized = serde_json::to_value(&updated).unwrap();
        assert!(serialized
            .get("remote_extension_manifest_revision")
            .is_none());
        serialized["remote_extension_manifest_revision"] = serde_json::json!(900);
        let restored: RuntimeProviderRun = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.remote_extension_manifest_revision(), 0);
        assert_eq!(
            restored.remote_extension_manifest(),
            updated.remote_extension_manifest()
        );
    }

    #[test]
    fn competing_manifest_compare_updates_have_exactly_one_winner() {
        let store = fixture();
        let start = Arc::new(std::sync::Barrier::new(3));
        let tasks = [false, true].map(|available| {
            let store = store.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                store
                    .compare_update_run_remote_extension_manifest("run", 0, manifest(available))
                    .unwrap()
                    .is_some()
            })
        });
        start.wait();
        let wins = tasks
            .into_iter()
            .map(|task| usize::from(task.join().unwrap()))
            .sum::<usize>();
        assert_eq!(wins, 1);
        assert_eq!(
            store
                .get_run("run")
                .unwrap()
                .remote_extension_manifest_revision(),
            1
        );
    }

    #[test]
    fn missing_run_manifest_compare_update_returns_error() {
        assert!(matches!(
            fixture().compare_update_run_remote_extension_manifest("missing", 0, manifest(true),),
            Err(DaemonError::ProviderRunNotFound { .. })
        ));
    }

    #[test]
    fn snapshot_restore_cannot_roll_manifest_revision_back() {
        let store = fixture();
        let mut snapshot = store.get_run("run").unwrap();
        snapshot.mark_running();
        store.write().insert_run_for_test(snapshot.clone());
        let pushed = store
            .update_run_remote_extension_manifest("run", manifest(true))
            .unwrap();
        let restored = store
            .restore_run_snapshot_after_restart_failure(snapshot)
            .unwrap();
        assert_eq!(restored.remote_extension_manifest_revision(), 2);
        assert!(store
            .compare_update_run_remote_extension_manifest(
                "run",
                pushed.remote_extension_manifest_revision(),
                manifest(true),
            )
            .unwrap()
            .is_none());
        assert_eq!(store.get_run("run").unwrap(), restored);
    }
}

impl std::fmt::Debug for ProviderProcessServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProcessServiceStore")
            .finish_non_exhaustive()
    }
}

impl ProviderProcessServiceStore {
    pub fn new(service: ProviderProcessService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(service)),
        }
    }

    pub fn read(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn write(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn registry(&self) -> ProviderRegistry {
        *self.read().registry()
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.read().run_operation_lanes()
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.read().set_native_interaction_bridge(bridge);
    }

    pub(crate) fn native_interaction_bridge(
        &self,
    ) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
        self.read().native_interaction_bridge()
    }

    pub(crate) fn run_actor_completion_signal(
        &self,
    ) -> crate::provider::ProviderRunActorCompletionSignal {
        self.read().run_actor_completion_signal()
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.read().get_run(run_id)
    }

    #[doc(hidden)]
    pub fn structured_runtime_state_bound_for_tests(&self, provider_run_id: &str) -> bool {
        self.read()
            .structured_runtime_state_bound_for_tests(provider_run_id)
    }

    pub(crate) fn start_run_provider_only(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        self.write().start_run_provider_only(request)
    }

    pub(crate) fn launch_run_detached(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().launch_run_detached(request)
    }

    pub(crate) fn park_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunParkedOutcome, DaemonError> {
        self.write().park_run_provider_only(session_id, run_id)
    }

    pub(crate) fn resume_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunResumedOutcome, DaemonError> {
        self.write().resume_run_provider_only(session_id, run_id)
    }

    pub fn resume_run_detached(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().resume_run_detached(run_id)
    }

    pub(crate) fn terminate_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        self.write().terminate_run_provider_only(session_id, run_id)
    }

    pub fn list_runs(&self) -> Vec<RuntimeProviderRun> {
        self.read().list_runs()
    }

    pub fn get_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_for_agent(session_id, agent_id)
    }

    pub fn get_latest_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_latest_run_for_agent(session_id, agent_id)
    }

    pub fn get_session_run_for_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read()
            .get_session_run_for_provider(session_id, provider)
    }

    pub fn get_run_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_by_runtime_mcp_auth_token(auth_token)
    }

    pub fn get_runs_by_runtime_mcp_auth_token(&self, auth_token: &str) -> Vec<RuntimeProviderRun> {
        self.read().get_runs_by_runtime_mcp_auth_token(auth_token)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.read().structured_prompt_io_in_flight(provider_run_id)
    }

    pub(crate) fn structured_runtime_state_bound(&self, provider_run_id: &str) -> bool {
        self.read().structured_runtime_state_bound(provider_run_id)
    }

    pub fn record_run_activity(&self, run_id: &str) -> Result<(), DaemonError> {
        self.write().record_run_activity(run_id)
    }

    pub(crate) fn mark_run_running(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().mark_run_running(run_id)
    }

    pub(crate) fn adapter_supports_turn_scoped_execution_config(&self, adapter_key: &str) -> bool {
        self.read()
            .adapter_supports_turn_scoped_execution_config(adapter_key)
    }

    pub(crate) fn update_run_execution_config(
        &self,
        run_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_execution_config(run_id, execution_mode, permission_level)
    }

    pub(crate) fn update_run_preparation_environment(
        &self,
        run_id: &str,
        home: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_preparation_environment(run_id, home, path)
    }

    pub(crate) fn update_run_read_only_discovery(
        &self,
        run_id: &str,
        enabled: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().update_run_read_only_discovery(run_id, enabled)
    }

    pub(crate) fn restore_run_snapshot_after_restart_failure(
        &self,
        snapshot: RuntimeProviderRun,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .restore_run_snapshot_after_restart_failure(snapshot)
    }

    pub(crate) fn mark_run_ended_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        self.write()
            .mark_run_ended_provider_only(session_id, run_id)
    }

    pub(crate) fn update_run_remote_extension_manifest(
        &self,
        run_id: &str,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_remote_extension_manifest(run_id, manifest)
    }

    pub(crate) fn compare_update_run_remote_extension_manifest(
        &self,
        run_id: &str,
        expected_revision: u64,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<Option<RuntimeProviderRun>, DaemonError> {
        self.write().compare_update_run_remote_extension_manifest(
            run_id,
            expected_revision,
            manifest,
        )
    }

    pub(crate) fn enable_workflow_tools(
        &self,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().enable_workflow_tools(run_id)
    }

    pub(crate) fn mark_workflow_fresh_context(
        &self,
        run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .mark_workflow_fresh_context(run_id, workflow_node_run_id)
    }

    pub(crate) fn reconcile_run_liveness_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        self.write()
            .reconcile_run_liveness_provider_only(session_id, run_id, process_running)
    }

    pub(crate) fn terminate_session_runs_provider_only(
        &self,
        session_id: &str,
    ) -> Result<ProviderSessionRunsTerminatedOutcome, DaemonError> {
        self.write()
            .terminate_session_runs_provider_only(session_id)
    }

    pub fn initialize_runtime(&self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        let binding = ProviderProcessService::initialize_runtime_binding(run)?;
        if let Some(binding) = binding {
            self.write().apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn initialize_runtime_with_credentials(
        &self,
        run: &RuntimeProviderRun,
        credentials: &crate::provider::ProviderCredentialEnvironment,
    ) -> Result<(), DaemonError> {
        let binding =
            ProviderProcessService::initialize_runtime_binding_with_credentials(run, credentials)?;
        if let Some(binding) = binding {
            self.write().apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn apply_runtime_binding(
        &self,
        run_id: &str,
        binding: ProviderRuntimeBinding,
    ) -> Result<(), DaemonError> {
        self.write().apply_runtime_binding(run_id, binding)
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        self.read().run_uses_structured_prompt_io(run)
    }

    pub fn enqueue_run_selection_sync(&self, provider_run_id: &str) -> Result<(), DaemonError> {
        self.write().enqueue_run_selection_sync(provider_run_id)
    }

    pub(crate) fn update_run_selection(
        &self,
        provider_run_id: &str,
        model: Option<String>,
        variant: Option<String>,
        clear_variant: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_selection(provider_run_id, model, variant, clear_variant)
    }

    pub(crate) fn record_observed_usage(
        &self,
        provider_run_id: &str,
        usage: ProviderRunTokenUsage,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().record_observed_usage(provider_run_id, usage)
    }

    pub(crate) fn apply_finished_provider_run_selection_sync_jobs(&self) {
        self.write()
            .apply_finished_provider_run_selection_sync_jobs()
    }

    pub fn clear_runtime(&self, provider_run_id: &str) {
        self.write().clear_runtime(provider_run_id)
    }

    pub(crate) fn enqueue_structured_prompt_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        origin_prompt_id: &str,
        run: &RuntimeProviderRun,
        prompt: &str,
        hidden_system_context: &str,
        attachments: &[PromptAttachment],
        mode: PromptAssemblyMode,
        steering: bool,
    ) -> Result<(), DaemonError> {
        self.write().enqueue_structured_prompt_submit(
            session_id,
            provider_run_id,
            agent_id,
            prompt_id,
            origin_prompt_id,
            run,
            prompt,
            hidden_system_context,
            attachments,
            mode,
            steering,
        )
    }

    pub(crate) fn run_structured_utility_prompt(
        &self,
        run: &RuntimeProviderRun,
        visible_user_prompt: &str,
        hidden_system_context: &str,
        timeout: std::time::Duration,
        policy: super::super::ProviderUtilityExecutionPolicy,
    ) -> Result<String, DaemonError> {
        self.write().run_structured_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
            policy,
        )
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        self.write()
            .enqueue_structured_prompt_abort(session_id, provider_run_id)
    }

    pub(crate) fn drain_finished_structured_prompt_submit_jobs(
        &self,
    ) -> Vec<FinishedProviderPromptSubmitJob> {
        self.write().drain_finished_structured_prompt_submit_jobs()
    }

    pub(crate) fn schedule_finished_structured_prompt_submit_retry(
        &self,
        finished: FinishedProviderPromptSubmitJob,
    ) {
        self.write()
            .schedule_finished_structured_prompt_submit_retry(finished);
    }

    pub(crate) fn schedule_finished_structured_output_poll_retry(
        &self,
        finished: FinishedProviderOutputPollJob,
    ) {
        self.write()
            .schedule_finished_structured_output_poll_retry(finished);
    }

    pub(crate) fn preview_structured_output_metadata(
        &self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.read()
            .preview_structured_output_metadata(provider_run_id, batch)
    }

    #[cfg(test)]
    pub(crate) fn push_finished_structured_prompt_submit_for_test(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        result: Result<crate::provider::ProviderPromptSubmitAcknowledgement, DaemonError>,
    ) {
        self.write()
            .push_finished_structured_prompt_submit_for_test(
                session_id,
                provider_run_id,
                agent_id,
                prompt_id,
                result,
            );
    }

    pub(crate) fn apply_prompt_submit_acknowledgement(
        &self,
        provider_run_id: &str,
        acknowledgement: &crate::provider::ProviderPromptSubmitAcknowledgement,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .apply_prompt_submit_acknowledgement(provider_run_id, acknowledgement)
    }

    pub(crate) fn drain_finished_structured_prompt_abort_jobs(
        &self,
    ) -> Vec<FinishedProviderPromptAbortJob> {
        self.write().drain_finished_structured_prompt_abort_jobs()
    }

    pub fn enqueue_structured_output_poll(
        &self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        self.write().enqueue_structured_output_poll(provider_run_id)
    }

    pub fn set_output_poll_delay_for_tests(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.read()
            .set_output_poll_delay_for_tests(provider_run_id, delay);
    }

    pub(crate) fn drain_finished_structured_output_poll_jobs(
        &self,
    ) -> Vec<FinishedProviderOutputPollJob> {
        self.write().drain_finished_structured_output_poll_jobs()
    }

    pub(crate) fn apply_structured_output_metadata(
        &self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        self.write()
            .apply_structured_output_metadata(provider_run_id, batch)
    }

    pub(crate) fn record_terminal_diagnostic(
        &self,
        provider_run_id: &str,
        diagnostic: impl Into<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .record_terminal_diagnostic(provider_run_id, diagnostic)
    }
}
