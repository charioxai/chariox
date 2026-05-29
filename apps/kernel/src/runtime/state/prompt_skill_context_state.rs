//! Granted-skill prompt context injection.
//!
//! This module owns adding granted Arroba skill summaries and explicitly requested skill bodies to
//! prompts before provider submission.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn apply_granted_skill_summary(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(prompt.to_string());
        }
        let session = self.session_store.get_session(session_id)?;
        let context = crate::skill::format_granted_skill_prompt_context(
            agent.agent_ref(),
            &skill_grants,
            session.workspace_id(),
            prompt,
        )?;
        if context.is_empty() {
            Ok(prompt.to_string())
        } else {
            Ok(format!("{context}\n\n{prompt}"))
        }
    }

    pub(super) fn granted_skill_hidden_context(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(String::new());
        }
        let session = self.session_store.get_session(session_id)?;
        crate::skill::format_granted_skill_prompt_context(
            agent.agent_ref(),
            &skill_grants,
            session.workspace_id(),
            prompt,
        )
    }
}
