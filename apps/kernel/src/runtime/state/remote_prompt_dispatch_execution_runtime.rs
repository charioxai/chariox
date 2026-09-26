//! One remote worker prompt submission and ACK handoff.

use super::remote_prompt_worker_submission_runtime::{
    remote_prompt_error_is_reconciliation_pending,
    submit_remote_prompt_to_worker_with_binding_refresh,
};
use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_remote_prompt_once(
        &self,
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) -> Option<crate::app::KernelRemotePromptDispatch> {
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
        let agent = match self.owned.agent_store.get_agent(&dispatch.agent_id) {
            Ok(agent) => agent,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return None;
            }
        };
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
        let attachments = dispatch.attachments.clone();
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
        let result = submit_remote_prompt_to_worker_with_binding_refresh(
            self,
            &mut dispatch,
            prompt,
            attachments,
        )
        .await;
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
