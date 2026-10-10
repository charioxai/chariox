use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;
use crate::runtime::command_latency::{log_command_completed, log_command_received, CommandTrace};
use crate::runtime::command_response_refresh::{
    refresh_command_response_state, CommandResponseRefreshContext,
};
use crate::runtime::session_membership::authorize_session_membership;
use crate::runtime::session_projection_refresh::{
    focus_projection_refresh, session_projection_refresh,
};

use super::CommandRouter;

impl CommandRouter {
    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_external_request(&command, &request)?;
        let mut router = self.clone();
        router.runtime_state = self
            .runtime_state
            .with_kernel_command_authority(&command, &request);
        if command.is_sudo_command() {
            router.runtime_state.authorize_current_external_command()?;
        }
        router.capability_runtime =
            crate::runtime::capability_executor::CapabilityRuntimeStore::new(
                router.runtime_state.clone(),
            );
        let response_command = command.clone();
        crate::runtime::external_response::finish_response(
            &response_command,
            router.dispatch_authorized(command, request).await,
        )
    }

    async fn dispatch_authorized(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let LocalDaemonRequest::RequestKernelSudo(sudo_request) = &request {
            crate::runtime::kernel_runtime_role_policy::ensure_public_request_allowed(
                &self.config_projection.snapshot(),
                &request,
            )?;
            if command.caller.connection_class
                != Some(crate::local::KernelConnectionClass::ExternalAgent)
            {
                return Err(crate::runtime::kernel_access::error(format!(
                    "only a grant holder over ws+unix://{} can request sudo",
                    self.kernel_local_socket_path().display()
                )));
            }
            return self
                .runtime_state
                .request_kernel_sudo(&command.caller.caller_id, sudo_request.clone())
                .await;
        }
        if let Some(response) = self.dispatch_kernel_access(&command, &request)? {
            return Ok(response);
        }
        self.audit_access_terminal_attempt(&command, &request)?;
        if let LocalDaemonRequest::RespondToInteraction(answer) = &request {
            if answer.session_id == crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE {
                if command.caller.connection_class
                    != Some(crate::local::KernelConnectionClass::Terminal)
                {
                    return Err(crate::runtime::kernel_access::error(
                        "only a terminal can answer access decisions",
                    ));
                }
                self.runtime_state
                    .answer_terminal_runtime_interaction(
                        &answer.session_id,
                        &answer.interaction_id,
                        &answer.choice_id,
                        answer.custom_reply.as_deref(),
                        Some(&crate::runtime::command::command_caller_user_id(&command)),
                        answer.passkey.as_ref(),
                        answer.passkey_remember_minutes,
                        command.caller.connection_class,
                    )
                    .await?;
                return Ok(LocalDaemonResponse::KernelAccessDecisionResponded {
                    interaction_id: answer.interaction_id.clone(),
                });
            }
        }
        let command_trace = CommandTrace::from_command(&command);
        log_command_received(&command_trace);
        if let Err(error) =
            crate::runtime::kernel_runtime_role_policy::ensure_public_request_allowed(
                &self.config_projection.snapshot(),
                &request,
            )
        {
            let result = Err(error);
            log_command_completed(&command_trace, &result);
            return result;
        }
        let focus_refresh = focus_projection_refresh(&request);
        let caller_user_id = match authorize_session_membership(
            &self.runtime_state,
            &self.session_projection,
            &command,
            &request,
        )
        .await
        {
            Ok(caller_user_id) => caller_user_id,
            Err(error) => {
                let result = Err(error);
                log_command_completed(&command_trace, &result);
                return result;
            }
        };
        if command.caller.connection_class == Some(crate::local::KernelConnectionClass::Terminal) {
            if let LocalDaemonRequest::RespondToInteraction(answer) = &request {
                if self
                    .runtime_state
                    .sudo_entry_pending(&answer.session_id, &answer.interaction_id)
                {
                    return self
                        .runtime_state
                        .answer_sudo_entry_from_terminal(
                            &crate::runtime::command::command_caller_user_id(&command),
                            &command.caller.caller_id,
                            answer,
                        )
                        .await;
                }
            }
        }
        // MP-08 / MP-10 / MP-11 A04: approvals belong to the user, including
        // when the requesting agent is elevated.
        if command.caller.connection_class == Some(crate::local::KernelConnectionClass::KernelAgent)
            && matches!(&request, LocalDaemonRequest::RespondToInteraction(_))
        {
            return Err(crate::runtime::kernel_access::error(
                "agents cannot answer approvals; the user answers in a Chariox terminal",
            ));
        }
        if let LocalDaemonRequest::ExtendKernelSudo(extend) = &request {
            if command.caller.connection_class
                != Some(crate::local::KernelConnectionClass::Terminal)
            {
                return Err(crate::runtime::kernel_access::error(
                    "only a Chariox terminal can extend sudo",
                ));
            }
            return self
                .runtime_state
                .extend_sudo_window(
                    extend.clone(),
                    &crate::runtime::command::command_caller_user_id(&command),
                )
                .await;
        }
        if let LocalDaemonRequest::SubmitPrompt(prompt) = &request {
            if crate::runtime::state::is_sudo_control(&prompt.prompt) {
                return Err(crate::runtime::kernel_access::error(
                    "use the terminal's sudo controls for status, extend or revoke",
                ));
            }
            if crate::runtime::state::is_sudo_prompt(&prompt.prompt) {
                if command.caller.connection_class
                    == Some(crate::local::KernelConnectionClass::ExternalAgent)
                {
                    return self
                        .runtime_state
                        .submit_external_sudo_prompt(&command.caller.caller_id, prompt.clone())
                        .await;
                }
                if command.caller.connection_class
                    != Some(crate::local::KernelConnectionClass::Terminal)
                {
                    return Err(crate::runtime::kernel_access::error(
                        "only a Chariox terminal can authorize sudo",
                    ));
                }
                return self
                    .runtime_state
                    .submit_sudo_prompt(
                        prompt.clone(),
                        &crate::runtime::command::command_caller_user_id(&command),
                        &command.caller.caller_id,
                    )
                    .await;
            }
        }
        if matches!(&request, LocalDaemonRequest::SubmitPrompts(batch) if batch.prompts.iter().any(|prompt| crate::runtime::state::is_sudo_prompt(&prompt.prompt) || crate::runtime::state::is_sudo_control(&prompt.prompt)))
        {
            return Err(crate::runtime::kernel_access::error(
                "submit /sudo individually so each entry has its own popup",
            ));
        }
        match self
            .dispatch_pre_lane(&command, &request, &caller_user_id)
            .await
        {
            Ok(Some(response)) => {
                if let Some((session_id, attachment_id)) =
                    projected_terminal_output_attachment(&request)
                {
                    if let Err(error) = self
                        .record_terminal_attachment_heartbeat(
                            session_id,
                            attachment_id,
                            crate::session::unix_epoch_ms(),
                        )
                        .await
                    {
                        let result = Err(error);
                        log_command_completed(&command_trace, &result);
                        return result;
                    }
                }
                let result = Ok(response);
                log_command_completed(&command_trace, &result);
                return result;
            }
            Ok(None) => {}
            Err(error) => {
                let result = Err(error);
                log_command_completed(&command_trace, &result);
                return result;
            }
        }

        let session_refresh = session_projection_refresh(&request);
        let result = self.dispatch_refresh_tracked(command, request).await;
        refresh_command_response_state(
            CommandResponseRefreshContext {
                app: &self.app,
                session_projection: &self.session_projection,
                agent_runtime_projection: &self.agent_runtime_projection,
                focus_projection: &self.focus_projection,
                provider_process_projection: &self.provider_process_projection,
                provider_launch_pending: &self.provider_launch_pending,
                provider_run_projection: &self.provider_run_projection,
                agent_runtime: &self.agent_runtime,
                workflow_runtime: &self.workflow_runtime,
            },
            session_refresh,
            focus_refresh,
            &result,
        )
        .await;
        let result = self.redact_result_for_user(result, &caller_user_id);
        log_command_completed(&command_trace, &result);
        result
    }
}

fn projected_terminal_output_attachment(request: &LocalDaemonRequest) -> Option<(&str, &str)> {
    match request {
        LocalDaemonRequest::PumpTerminalOutput(request) => {
            Some((&request.session_id, &request.attachment_id))
        }
        _ => None,
    }
}
