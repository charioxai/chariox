use crate::error::DaemonError;
use crate::prompt_assembly::{PromptAssemblyMode, PromptAssemblyService};
use crate::provider::{
    CodexRunSelection, FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob,
    FinishedProviderPromptSubmitJob, ProviderPromptSignalBatch, ProviderRunOperationLanes,
    ProviderRunTokenUsage, RuntimeProviderRun,
};
use crate::session::PromptAttachment;

use super::super::{
    claude_runtime::{ClaudeRunSelection, ClaudeRuntimeBinding, initialize_claude_runtime},
    codex_runtime::{CodexRuntimeBinding, initialize_codex_runtime},
    opencode_binding::{OpenCodeRunSelection, OpenCodeRuntimeBinding, initialize_opencode_runtime},
};
use super::ProviderProcessService;

pub(crate) enum ProviderRuntimeBinding {
    Claude(ClaudeRuntimeBinding),
    Codex(CodexRuntimeBinding),
    OpenCode(OpenCodeRuntimeBinding),
}

impl ProviderProcessService {
    pub fn initialize_runtime(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if let Some(binding) = Self::initialize_runtime_binding(run)? {
            self.apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn initialize_runtime_binding(
        run: &RuntimeProviderRun,
    ) -> Result<Option<ProviderRuntimeBinding>, DaemonError> {
        if run.adapter_key() == "dev-stub" && run.provider() == "runtime-init-fail" {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "dev_stub_runtime_init",
                message: "forced dev-stub runtime initialization failure".to_string(),
            });
        }
        if run.adapter_key() == "codex" {
            return initialize_codex_runtime(run)
                .map(ProviderRuntimeBinding::Codex)
                .map(Some);
        }
        if run.adapter_key() == "claude"
            && run.client_interface().is_arroba()
            && !crate::provider::provider_run_uses_claude_native_bridge(run)
        {
            return initialize_claude_runtime(run)
                .map(ProviderRuntimeBinding::Claude)
                .map(Some);
        }
        if run.adapter_key() == "opencode" {
            return initialize_opencode_runtime(run)
                .map(ProviderRuntimeBinding::OpenCode)
                .map(Some);
        }
        Ok(None)
    }

    pub(crate) fn apply_runtime_binding(
        &mut self,
        run_id: &str,
        binding: ProviderRuntimeBinding,
    ) -> Result<(), DaemonError> {
        match binding {
            ProviderRuntimeBinding::Claude(binding) => {
                self.run_actor_mailbox
                    .insert_claude_runtime(run_id.to_string(), binding.state);
                self.apply_claude_run_selection(run_id, binding.selection)?;
            }
            ProviderRuntimeBinding::Codex(binding) => {
                self.run_actor_mailbox
                    .insert_codex_runtime(run_id.to_string(), binding.state);
                self.apply_codex_run_selection(run_id, binding.selection)?;
            }
            ProviderRuntimeBinding::OpenCode(binding) => {
                self.run_actor_mailbox
                    .insert_opencode_runtime(run_id.to_string(), binding.state);
                let run_mut = self.get_run_mut(run_id)?;
                run_mut.set_resume_state(binding.resume_state.clone());
                run_mut.set_provider_session_id(
                    binding
                        .resume_state
                        .opencode_session_id()
                        .map(str::to_string),
                );
                self.apply_opencode_run_selection(run_id, binding.selection)?;
            }
        }
        Ok(())
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        run.adapter_key() == "codex"
            || (run.adapter_key() == "claude"
                && run.client_interface().is_arroba()
                && !crate::provider::provider_run_uses_claude_native_bridge(run))
            || run.adapter_key() == "opencode"
            || (run.adapter_key() == "dev-stub" && run.provider() == "slow-structured")
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.run_actor_mailbox
            .structured_prompt_io_in_flight(provider_run_id)
    }

    #[doc(hidden)]
    pub fn structured_runtime_state_bound_for_tests(&self, provider_run_id: &str) -> bool {
        self.run_actor_mailbox
            .structured_runtime_state_bound(provider_run_id)
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.run_actor_mailbox.operation_lanes()
    }

    pub fn enqueue_run_selection_sync(&mut self, provider_run_id: &str) -> Result<(), DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() != "opencode" {
            return Ok(());
        }
        self.run_actor_mailbox
            .spawn_selection_sync(provider_run_id.to_string(), run)
    }

    pub(crate) fn apply_finished_provider_run_selection_sync_jobs(&mut self) {
        for finished in self.run_actor_mailbox.drain_finished_selection_syncs() {
            match finished.result {
                Ok(selection) => {
                    let Ok(current_run) = self.get_run(&finished.provider_run_id) else {
                        continue;
                    };
                    let selection = Self::merge_opencode_run_selection(&current_run, selection);
                    if let Err(error) =
                        self.apply_opencode_run_selection(&finished.provider_run_id, selection)
                    {
                        crate::logging::error_with_fields(
                            "daemon.provider",
                            "provider run selection sync apply failed",
                            serde_json::json!({
                                "provider_run_id": finished.provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                Err(error) => {
                    crate::logging::debug_with_fields(
                        "daemon.provider",
                        "provider run selection sync failed",
                        serde_json::json!({
                            "provider_run_id": finished.provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }

    pub fn clear_runtime(&mut self, provider_run_id: &str) {
        self.run_actor_mailbox.clear_runtime(provider_run_id);
        self.run_actor_mailbox.stop_run(provider_run_id);
    }

    pub(crate) fn enqueue_structured_prompt_submit(
        &mut self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: &RuntimeProviderRun,
        prompt: &str,
        hidden_system_context: &str,
        attachments: &[PromptAttachment],
        mode: PromptAssemblyMode,
    ) -> Result<(), DaemonError> {
        let _ = self.record_run_activity(run.id());
        if !self.run_uses_structured_prompt_io(run) {
            return Err(DaemonError::LocalTransport {
                operation: "enqueue structured prompt dispatch",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            });
        }
        let mode = if mode == PromptAssemblyMode::MetaagentProviderTurn {
            mode
        } else if run.client_interface().is_arroba() {
            PromptAssemblyMode::NormalProviderTurn
        } else {
            PromptAssemblyMode::NativeTuiProviderTurn
        };
        let envelope = PromptAssemblyService::from_env()?.assemble_provider_turn(
            run,
            prompt,
            Some(hidden_system_context),
            attachments.to_vec(),
            mode,
        )?;
        self.run_actor_mailbox.spawn_submit(
            session_id,
            provider_run_id,
            agent_id,
            run.clone(),
            envelope,
        )
    }

    pub(crate) fn run_structured_utility_prompt(
        &mut self,
        run: &RuntimeProviderRun,
        visible_user_prompt: &str,
        hidden_system_context: &str,
        timeout: std::time::Duration,
    ) -> Result<String, DaemonError> {
        if !self.run_uses_structured_prompt_io(run) {
            return Err(DaemonError::LocalTransport {
                operation: "run structured utility prompt",
                message: format!(
                    "provider run `{}` does not use structured prompt I/O",
                    run.id()
                ),
            });
        }
        let envelope = PromptAssemblyService::from_env()?.assemble_provider_turn(
            run,
            visible_user_prompt,
            Some(hidden_system_context),
            Vec::new(),
            PromptAssemblyMode::UtilityTurn,
        )?;
        self.run_actor_mailbox
            .run_utility(run.id().to_string(), run.clone(), envelope, timeout)
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &mut self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        let run = self.get_run(&provider_run_id)?;
        if !self.run_uses_structured_prompt_io(&run) {
            return Err(DaemonError::LocalTransport {
                operation: "enqueue structured prompt abort",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            });
        }
        self.run_actor_mailbox
            .spawn_abort(session_id, provider_run_id, run)
    }

    pub(crate) fn drain_finished_structured_prompt_submit_jobs(
        &mut self,
    ) -> Vec<FinishedProviderPromptSubmitJob> {
        self.run_actor_mailbox.drain_finished_submits()
    }

    pub(crate) fn drain_finished_structured_prompt_abort_jobs(
        &mut self,
    ) -> Vec<FinishedProviderPromptAbortJob> {
        self.run_actor_mailbox.drain_finished_aborts()
    }

    pub fn enqueue_structured_output_poll(
        &mut self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let _ = self.record_run_activity(provider_run_id);
        let run = self.get_run(provider_run_id)?;
        if !self.run_uses_structured_prompt_io(&run) {
            return Ok(false);
        }
        self.run_actor_mailbox
            .spawn_output_poll(provider_run_id.to_string(), run)
    }

    #[doc(hidden)]
    pub fn set_output_poll_delay_for_tests(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.run_actor_mailbox
            .set_output_poll_delay_for_tests(provider_run_id, delay);
    }

    pub(crate) fn drain_finished_structured_output_poll_jobs(
        &mut self,
    ) -> Vec<FinishedProviderOutputPollJob> {
        self.run_actor_mailbox.drain_finished_output_polls()
    }

    #[cfg(test)]
    pub(crate) fn push_finished_structured_output_poll_for_test(
        &mut self,
        provider_run_id: String,
        result: Result<Option<ProviderPromptSignalBatch>, DaemonError>,
    ) {
        self.run_actor_mailbox
            .push_finished_output_poll_for_test(FinishedProviderOutputPollJob {
                provider_run_id,
                result,
            });
    }

    #[cfg(test)]
    pub(crate) fn insert_run_for_test(&mut self, run: RuntimeProviderRun) {
        self.runs.insert(run.id().to_string(), run);
    }

    pub(crate) fn apply_structured_output_metadata(
        &mut self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let has_terminal_diagnostic = batch
            .terminal_failure
            .as_deref()
            .is_some_and(|message| !message.trim().is_empty());
        if batch.resolved_model.is_none()
            && batch.resolved_variant.is_none()
            && batch.resolved_usage_tokens_total.is_none()
            && batch.resolved_usage.is_none()
            && batch.resolved_resume_state.is_none()
            && !has_terminal_diagnostic
        {
            return Ok(());
        }
        let run = self.get_run_mut(provider_run_id)?;
        let adapter_key = run.adapter_key().to_string();
        if let Some(message) = batch.terminal_failure.as_deref() {
            if !message.trim().is_empty() {
                run.set_terminal_diagnostic(message.to_string());
            }
        }
        if let Some(model) = batch.resolved_model.as_deref() {
            let model = if adapter_key == "claude" {
                normalize_claude_selection_model(model)
            } else {
                model.to_string()
            };
            crate::logging::debug_with_fields(
                "daemon.provider.opencode",
                "resolved provider run model from opencode metadata",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "previous_model": run.model(),
                    "resolved_model": model,
                    "source": batch.resolved_model_source,
                }),
            );
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = batch.resolved_variant.as_deref() {
            if run.variant() != Some(variant) {
                crate::logging::debug_with_fields(
                    "daemon.provider.opencode",
                    "resolved provider run variant from opencode metadata",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "previous_variant": run.variant(),
                        "resolved_variant": variant,
                    }),
                );
                run.set_variant(Some(variant.to_string()));
            }
        }
        if let Some(total_tokens) = batch.resolved_usage_tokens_total {
            if run.usage_tokens_total() != Some(total_tokens) {
                run.set_usage_tokens_total(Some(total_tokens));
            }
        }
        if let Some(usage) = batch.resolved_usage.or_else(|| {
            batch
                .resolved_usage_tokens_total
                .map(ProviderRunTokenUsage::from_total_tokens)
        }) {
            if run.usage() != usage {
                run.set_usage(usage);
            }
        }
        if let Some(resume_state) = batch.resolved_resume_state.as_ref() {
            run.set_resume_state(resume_state.clone());
            run.set_provider_session_id(
                resume_state
                    .opencode_session_id()
                    .or_else(|| resume_state.codex_thread_id())
                    .or_else(|| resume_state.claude_session_id())
                    .map(str::to_string),
            );
        }
        Ok(())
    }

    pub(crate) fn record_terminal_diagnostic(
        &mut self,
        provider_run_id: &str,
        diagnostic: impl Into<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        run.set_terminal_diagnostic(diagnostic);
        Ok(run.clone())
    }

    pub(crate) fn update_run_selection(
        &mut self,
        provider_run_id: &str,
        model: Option<String>,
        variant: Option<String>,
        clear_variant: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if run.model() != model {
                run.set_model(model);
            }
        }
        if clear_variant {
            if run.variant().is_some() {
                run.set_variant(None);
            }
        } else if let Some(variant) = variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(run.clone())
    }

    fn apply_opencode_run_selection(
        &mut self,
        provider_run_id: &str,
        selection: OpenCodeRunSelection,
    ) -> Result<(), DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = selection.model {
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = selection.variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(())
    }

    fn apply_claude_run_selection(
        &mut self,
        provider_run_id: &str,
        selection: ClaudeRunSelection,
    ) -> Result<(), DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = selection.model {
            let model = normalize_claude_selection_model(&model);
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = selection.variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(())
    }

    fn apply_codex_run_selection(
        &mut self,
        provider_run_id: &str,
        selection: CodexRunSelection,
    ) -> Result<(), DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = selection.model {
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = selection.variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(())
    }

    pub(super) fn merge_opencode_run_selection(
        run: &RuntimeProviderRun,
        selection: OpenCodeRunSelection,
    ) -> OpenCodeRunSelection {
        OpenCodeRunSelection {
            model: selection.model.or_else(|| Some(run.model().to_string())),
            variant: selection
                .variant
                .or_else(|| run.variant().map(str::to_string)),
        }
    }
}

fn normalize_claude_selection_model(model: &str) -> String {
    model
        .trim()
        .split('/')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_claude_selection_model;

    #[test]
    fn claude_selection_models_stay_provider_native() {
        assert_eq!(
            normalize_claude_selection_model("claude/claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_claude_selection_model("claude/claude/claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_claude_selection_model("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
    }
}
