use std::fs;

use base64::Engine;

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
        attachments
            .into_iter()
            .enumerate()
            .map(|(index, attachment)| {
                if let Some(contents_base64) = attachment.contents_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(contents_base64)
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "decode remote prompt attachment",
                            message: error.to_string(),
                        })?;
                    let filename = attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| format!("attachment-{index}"));
                    let root = std::env::temp_dir()
                        .join("arroba-remote-prompt-attachments")
                        .join(&leased_agent.backing_session_id)
                        .join(&leased_agent.id);
                    fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
                        operation: "create remote prompt attachment directory",
                        message: error.to_string(),
                    })?;
                    let path = root.join(format!(
                        "{}-{}-{}",
                        crate::session::unix_epoch_ms(),
                        index,
                        filename
                    ));
                    fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "write remote prompt attachment",
                        message: error.to_string(),
                    })?;
                    Ok(crate::session::PromptAttachment::new(
                        format!("file://{}", path.display()),
                        attachment.mime,
                        Some(filename),
                    ))
                } else {
                    Ok(crate::session::PromptAttachment::new(
                        attachment.url,
                        attachment.mime,
                        attachment.filename,
                    ))
                }
            })
            .collect()
    }
}
