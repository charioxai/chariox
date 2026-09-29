//! One remote worker prompt submission and ACK handoff.

use super::remote_prompt_worker_submission_runtime::{
    remote_prompt_error_is_reconciliation_pending,
    submit_remote_prompt_to_worker_with_binding_refresh,
};
use super::*;

#[cfg(test)]
static REMOTE_PROMPT_DISPATCH_TEST_STAGES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<String, Vec<&'static str>>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
pub(super) fn record_remote_prompt_dispatch_test_stage(prompt_id: &str, stage: &'static str) {
    const MAX_PROMPTS: usize = 4096;
    const MAX_STAGES_PER_PROMPT: usize = 24;

    let stages = REMOTE_PROMPT_DISPATCH_TEST_STAGES
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let Ok(mut stages) = stages.lock() else {
        return;
    };
    if !stages.contains_key(prompt_id) && stages.len() >= MAX_PROMPTS {
        return;
    }
    let prompt_stages = stages.entry(prompt_id.to_string()).or_default();
    if prompt_stages.len() < MAX_STAGES_PER_PROMPT {
        prompt_stages.push(stage);
    }
}

#[cfg(test)]
pub(super) fn remote_prompt_dispatch_test_stages(prompt_id: &str) -> String {
    REMOTE_PROMPT_DISPATCH_TEST_STAGES
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .ok()
        .and_then(|stages| stages.get(prompt_id).cloned())
        .unwrap_or_default()
        .join(" > ")
}

impl KernelRuntimeState {
    pub(super) async fn dispatch_remote_prompt_once(
        &self,
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) -> Option<crate::app::KernelRemotePromptDispatch> {
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "dispatch_once_entered");
        crate::logging::info_with_fields(
            "daemon.remote_prompt_dispatch",
            "remote prompt dispatch starting",
            serde_json::json!({
                "session_id": dispatch.session_id,
                "agent_id": dispatch.agent_id,
                "worker_kernel_id": dispatch.worker_kernel_id,
                "leased_agent_id": dispatch.leased_agent_id,
                "source_attachment_id": dispatch.source_attachment_id,
            }),
        );
        if let Err(error) = self.owned.mark_active_prompt_delivery(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        ) {
            let _ = self
                .finish_remote_prompt_dispatch(dispatch, Err(error))
                .await;
            return None;
        }
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "durable_dispatching_marked");
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "agent_lookup_started");
        let agent = match self.owned.agent_store.get_agent(&dispatch.agent_id) {
            Ok(agent) => agent,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return None;
            }
        };
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "agent_lookup_ready");
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "skill_context_started");
        let (prompt, _) = match self
            .prepare_remote_prompt_skill_context(&agent, &dispatch.prompt)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return None;
            }
        };
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "skill_context_ready");
        let attachments = dispatch.attachments.clone();
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(
            &dispatch.prompt_id,
            "attachment_serialization_started",
        );
        let serialized_attachments = match tokio::task::spawn_blocking(move || {
            crate::app::serialize_remote_prompt_attachments(&attachments)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(DaemonError::LocalTransport {
                operation: "serialize remote prompt attachments",
                message: error.to_string(),
            }),
        };
        let attachments = match serialized_attachments {
            Ok(attachments) => attachments,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return None;
            }
        };
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "attachments_serialized");
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "worker_submission_started");
        let result = submit_remote_prompt_to_worker_with_binding_refresh(
            self,
            &mut dispatch,
            prompt,
            attachments,
        )
        .await;
        #[cfg(test)]
        record_remote_prompt_dispatch_test_stage(&dispatch.prompt_id, "worker_submission_returned");
        match &result {
            Ok(provider_run_id) => crate::logging::info_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch submitted",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "remote_provider_run_id": provider_run_id,
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch failed",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
        if result
            .as_ref()
            .err()
            .is_some_and(remote_prompt_error_is_reconciliation_pending)
        {
            // An indeterminate submission must keep the same durable prompt active.
            // The ordinary error finalizer would cancel it and promote the backlog.
            return Some(dispatch);
        }
        match result {
            Ok(remote_provider_run_id) => {
                if let Err(error) = self
                    .finish_remote_prompt_dispatch(dispatch.clone(), Ok(remote_provider_run_id))
                    .await
                {
                    let message = format!(
                        "Worker accepted prompt `{}`, but the home kernel could not record its acknowledgement: {error}. Delivery is uncertain; do not replay this prompt without checking the worker.",
                        dispatch.prompt_id,
                    );
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        &message,
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "delivery_uncertain": true,
                        }),
                    );
                    self.owned.fan_out_remote_dispatch_error(
                        &dispatch,
                        &format!("remote-dispatch:{}", dispatch.prompt_id),
                        Some(format!(
                            "remote-dispatch-ack-persist-failed:{}",
                            dispatch.prompt_id
                        )),
                        &message,
                    );
                    return Some(dispatch);
                }
            }
            Err(error) => {
                if let Err(settlement_error) = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt failure could not be settled",
                        serde_json::json!({"error": settlement_error.to_string()}),
                    );
                }
            }
        }
        None
    }
}
