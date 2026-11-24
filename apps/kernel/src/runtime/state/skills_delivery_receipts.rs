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
    /// Claude's prompt acknowledgement only proves the PTY write; its hook receipt
    /// is the official confirmation that the instruction bodies were consumed.
    pub(super) fn record_hook_receipts(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
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
        if batch.hook_context_delivered {
            // A structured Claude run has one turn in flight: the one whose hook reported.
            let session = self.sessions.get_session(run.session_id())?;
            if let Some(prompt) = self.prompts.active_prompt_for_agent(&session, agent_id) {
                self.record_hook_receipts(run, run.session_id(), agent_id, prompt.id())?;
            }
        }
        if crate::provider::canonical_provider_family(run.provider()) == Some("opencode") {
            self.record_native_tool_receipts(run, agent_id, batch)?;
        }
        for ack in &batch.skill_tool_delivery_acknowledgements {
            let session = self.sessions.get_session(run.session_id())?;
            let leased = self
                .prompts
                .active_prompt_for_agent(&session, agent_id)
                .is_some_and(|prompt| prompt.leased_skill_context().is_some());
            if self.runs.is_leased_provider_run(run.id()) || leased {
                self.forward_leased_delivery(
                    run,
                    run.session_id(),
                    agent_id,
                    &ack.prompt_id,
                    leased_receipts::LeasedSkillAcknowledgement::Tool {
                        skill_id: ack.skill_id.clone(),
                        digest: ack.digest.clone(),
                    },
                )?;
                continue;
            }
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
        if run.session_id() != session_id || run.agent_instance_id() != Some(agent_id) {
            return Ok(());
        }
        let session = self.sessions.get_session(session_id)?;
        let leased = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.leased_skill_context().is_some());
        // A leased receipt belongs to the home catalog, which a second named worker
        // does not own: route it before asking this kernel for a catalog.
        if self.runs.is_leased_provider_run(run.id()) || leased {
            return self.forward_leased_delivery(
                run,
                session_id,
                agent_id,
                prompt_id,
                leased_receipts::LeasedSkillAcknowledgement::Prompt,
            );
        }
        if !super::super::skills_catalog_runtime::catalog_available(&self.durable) {
            return Ok(());
        }
        self.record_pending_loads(
            run.id(),
            run.id(),
            session_id,
            agent_id,
            prompt_id,
            admitted,
        )
    }
    /// `run_id` finds the turn's authority; `stored_run_id` is the provider run the
    /// delivered load belongs to (a leased turn is admitted before its worker run exists).
    pub(super) fn record_pending_loads(
        &self,
        run_id: &str,
        stored_run_id: &str,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        admitted: impl Fn(&SkillTurnAuthority, &SkillLoadRecord) -> bool,
    ) -> Result<(), DaemonError> {
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
            .filter(|authority| authority.provider_run_id == run_id)
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
                    current.provider_run_id == run_id && current.context_generation == generation
                }) else {
                    return Ok(());
                };
                // record_load is idempotent by agent/run/generation/ID/full digest.
                // If fsync fails, the existing provider completion queue retains
                // this ACK and retries settlement without re-submitting the task.
                let mut delivered = false;
                for record in records {
                    let reload = (record.skill_id.clone(), record.digest.clone());
                    if current.pending_loads.iter().any(|pending| {
                        pending.skill_id == record.skill_id
                            && pending.digest == record.digest
                            && pending.run_id == record.run_id
                            && pending.prompt_id == record.prompt_id
                            && pending.context_generation == record.context_generation
                    }) {
                        let stored = SkillLoadRecord {
                            run_id: stored_run_id.into(),
                            ..record
                        };
                        service.storage.record_load_at_revision(
                            &stored,
                            Some(&current.project_id),
                            current.catalog_revision,
                        )?;
                        self.changes.record_change();
                        delivered = true;
                        if let Some(revision) = current.compaction_revision {
                            self.history.record_agent_compaction_reload(
                                session_id, agent_id, revision, &reload.0, &reload.1,
                            )?;
                        }
                    }
                }
                // A compaction lost every earlier load, so the epoch stays open until the
                // harness has taken the reload of each skill this turn re-attached, whichever
                // turn acknowledged it; a skill still pending would otherwise look held.
                let Some(revision) = current.compaction_revision.filter(|_| delivered) else {
                    return Ok(());
                };
                let reloaded = self
                    .history
                    .agent_compaction_reloads(session_id, agent_id, revision)?;
                let every_reload_acknowledged = current.pending_loads.iter().all(|pending| {
                    reloaded.contains(&(pending.skill_id.clone(), pending.digest.clone()))
                });
                if every_reload_acknowledged {
                    self.history
                        .deliver_agent_compaction_boundary(session_id, agent_id, revision)
                } else {
                    Ok(())
                }
            })
    }
    /// OpenCode loads a deduped or `From OpenCode` skill with its own `skill` tool; the
    /// completed call settles the pending load the turn mapped to that name.
    fn record_native_tool_receipts(
        &self,
        run: &RuntimeProviderRun,
        agent_id: &str,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let names: Vec<String> = batch
            .chunks
            .iter()
            .filter_map(|chunk| {
                crate::provider::completed_native_skill_tool(chunk.kind.clone(), &chunk.bytes)
            })
            .collect();
        if names.is_empty() {
            return Ok(());
        }
        let session = self.sessions.get_session(run.session_id())?;
        let Some(prompt) = self.prompts.active_prompt_for_agent(&session, agent_id) else {
            return Ok(());
        };
        if prompt.leased_skill_context().is_some() {
            // The worker's own run reports the call; it holds no authority, so the home
            // settles the load the name maps to.
            return names.into_iter().try_for_each(|name| {
                self.forward_leased_delivery(
                    run,
                    run.session_id(),
                    agent_id,
                    prompt.id(),
                    leased_receipts::LeasedSkillAcknowledgement::NativeTool { name },
                )
            });
        }
        let skill_ids: std::collections::BTreeSet<String> = prompt
            .skill_turn_authority()
            .map(|authority| {
                names
                    .iter()
                    .filter_map(|name| authority.native_tool_loads.get(name).cloned())
                    .collect()
            })
            .unwrap_or_default();
        if skill_ids.is_empty() {
            return Ok(());
        }
        self.record_acknowledged_loads(run, run.session_id(), agent_id, prompt.id(), |_, record| {
            skill_ids.contains(&record.skill_id)
        })
    }
}
