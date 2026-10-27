use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::transport::relay_peer::RelayPromptAttachment;

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    #[cfg(test)]
    pub(crate) fn leased_agent_active_prompt_attachments(
        &self,
        leased_agent_id: &str,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        Ok(self
            .app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .map(|prompt| prompt.attachments().to_vec())
            .unwrap_or_default())
    }

    pub(super) fn materialize_leased_prompt_attachments(
        &self,
        leased_agent: &LeasedAgent,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
        let attachments = attachments
            .into_iter()
            .map(|attachment| {
                let value = crate::session::PromptAttachment::new(
                    attachment.url,
                    attachment.mime,
                    attachment.filename,
                );
                match attachment.contents_base64 {
                    Some(contents) => value.with_contents_base64(contents),
                    None => value,
                }
            })
            .collect();
        crate::runtime::agent_actor::prompt_attachment_materialization::materialize_inline_prompt_attachments(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            attachments,
        )
    }
}
