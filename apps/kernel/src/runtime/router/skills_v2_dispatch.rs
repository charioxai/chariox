//! MP-08 / MP-10 / MP-11: source-aware submissions reuse the ordinary agent lane.
use super::CommandRouter;
use crate::{
    error::DaemonError,
    local::{LocalDaemonResponse, SubmitPromptRequest},
    runtime::command::KernelCommand,
    skill::v2::{SkillsV2Request, SkillsV2Response},
};
impl CommandRouter {
    pub(super) async fn dispatch_skills_v2(
        &self,
        command: &KernelCommand,
        request: SkillsV2Request,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let SkillsV2Request::Submit {
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
            selections,
        } = request
        {
            self.runtime_state.ensure_skills_v2_catalog_ready()?;
            let response = self
                .agent_runtime
                .dispatch_prompt_submit_with_skills(
                    command,
                    SubmitPromptRequest {
                        session_id,
                        attachment_id,
                        target_agent_id: Some(target_agent_id),
                        prompt,
                        attachments,
                    },
                    selections,
                )
                .await?;
            match response {
                LocalDaemonResponse::PromptSubmitted { outcome, .. } => {
                    Ok(LocalDaemonResponse::SkillsV2 {
                        result: SkillsV2Response::Submitted { outcome },
                    })
                }
                _ => Err(DaemonError::LocalTransport {
                    operation: "skills.v2.submit",
                    message: "ordinary prompt admission returned an unexpected response".into(),
                }),
            }
        } else {
            self.runtime_state
                .with_skills_catalog_caller(command)
                .execute_skills_catalog_request(request, caller_user_id.to_owned())
                .await
        }
    }
}
