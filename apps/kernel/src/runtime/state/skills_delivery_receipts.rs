//! MP-08 / MP-10 / MP-11: only official delivery acknowledgements become Loaded.
use super::*;
impl SkillDispatchService {
    pub(super) fn record_prompt_receipts(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        // Claude's structured submit acknowledges an SDK pipe write, not
        // additionalContext consumption. Its hook/stream receipt must settle
        // these pending bodies; availability must never masquerade as Loaded.
        if crate::provider::canonical_provider_family(run.provider()) == Some("claude") {
            return Ok(());
        }
        self.record_acknowledged_loads(run, session_id, agent_id, prompt_id, |authority, record| {
            authority
                .prompt_delivery_skill_ids
                .contains(&record.skill_id)
        })
    }
    pub(super) fn record_tool_receipts(
        &self,
        run: &RuntimeProviderRun,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        for ack in &batch.skill_tool_delivery_acknowledgements {
            self.record_acknowledged_loads(
                run,
                run.session_id(),
                agent_id,
                &ack.prompt_id,
                |_, record| record.skill_id == ack.skill_id && record.digest == ack.digest,
            )?;
        }
        Ok(())
    }
    fn record_acknowledged_loads(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        admitted: impl Fn(&SkillTurnAuthority, &SkillLoadRecord) -> bool,
    ) -> Result<(), DaemonError> {
        if !super::super::skills_catalog_runtime::SKILLS_V2_RUNTIME_INTEGRATED {
            return Ok(());
        }
        if run.session_id() != session_id || run.agent_instance_id() != Some(agent_id) {
            return Ok(());
        }
        // Lease results are translated by the home binding, never recorded by a
        // worker's unrelated local catalog. Transfer integration supplies that seam.
        if self.runs.is_leased_provider_run(run.id()) {
            return Ok(());
        }
        let session = self.sessions.get_session(session_id)?;
        let Some(prompt) = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| prompt.id() == prompt_id)
        else {
            return Ok(());
        };
        let Some(authority) = prompt
            .skill_turn_authority()
            .filter(|authority| authority.provider_run_id == run.id())
        else {
            return Ok(());
        };
        let service = self.catalog()?;
        let mut records: Vec<_> = authority
            .pending_loads
            .iter()
            .filter(|record| admitted(authority, record))
            .cloned()
            .collect();
        if records.is_empty() {
            return Ok(());
        }
        let now = crate::session::unix_epoch_ms().max(1);
        for record in &mut records {
            record.delivered_at_ms = now
        }
        let generation = authority.context_generation.clone();
        self.prompts
            .with_active_prompt(&session, agent_id, |current| {
                let Some(current) = current.filter(|current| {
                    current.id() == prompt_id
                        && matches!(
                            current.status(),
                            crate::session::PromptStatus::Running
                                | crate::session::PromptStatus::Dispatching
                        )
                }) else {
                    return Ok(());
                };
                let Some(current) = current.skill_turn_authority().filter(|current| {
                    current.provider_run_id == run.id() && current.context_generation == generation
                }) else {
                    return Ok(());
                };
                // record_load is idempotent by agent/run/generation/ID/full digest.
                // If fsync fails, the existing provider completion queue retains
                // this ACK and retries settlement without re-submitting the task.
                for record in records {
                    if current.pending_loads.iter().any(|pending| {
                        pending.skill_id == record.skill_id
                            && pending.digest == record.digest
                            && pending.run_id == record.run_id
                            && pending.prompt_id == record.prompt_id
                            && pending.context_generation == record.context_generation
                    }) {
                        service.storage.record_load_at_revision(
                            &record,
                            Some(&current.project_id),
                            current.catalog_revision,
                        )?;
                        self.changes.record_change();
                    }
                }
                Ok(())
            })
    }
}
