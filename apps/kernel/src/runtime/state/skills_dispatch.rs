//! MP-08 / MP-10 / MP-11: one instruction preparation service for every structured submission.
use super::*;
#[path = "skills_delivery_receipts.rs"]
mod receipts;
use crate::{
    prompt_assembly::PromptEnvelope,
    provider::{
        skill_delivery::{ProviderSkillDelivery, SkillPromptBridge},
        RuntimeProviderRun,
    },
    skill::{
        catalog::SkillCatalogSet, service::SkillCatalogService, storage::SkillCatalogStorage,
        turn_authority::SkillTurnAuthority, v2::*,
    },
};

/// Store handles only: this bridge deliberately holds no ProviderProcessServiceStore
/// and cannot form a callback cycle through the provider mailbox.
struct SkillDispatchService {
    changes: Arc<RuntimeChangeSignal>,
    config: crate::runtime::projection::DaemonConfigProjectionStore,
    durable: DurableKernelStateStore,
    sessions: SessionStateStore,
    agents: AgentServiceStore,
    prompts: crate::runtime::prompt_state::PromptStateOwner,
    runs: crate::runtime::projection::ProviderRunProjectionStore,
    inventories: Arc<super::skills_native_inventory::SkillInventories>,
}
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.dispatch",
        message: message.into(),
    }
}
pub(super) fn bridge(state: &KernelRuntimeState) -> Arc<dyn SkillPromptBridge> {
    let owned = &state.owned;
    Arc::new(SkillDispatchService {
        changes: owned.runtime_projection_changes.clone(),
        config: owned.config_projection.clone(),
        durable: owned.durable_state_store.clone(),
        sessions: owned.session_store.clone(),
        agents: owned.agent_store.clone(),
        prompts: owned.prompt_state_owner.clone(),
        runs: owned.provider_run_projection.clone(),
        inventories: owned.skill_inventories.clone(),
    })
}
impl SkillDispatchService {
    fn catalog(&self) -> Result<SkillCatalogService, DaemonError> {
        if !self.durable.owns_skill_catalog() || !self.durable.skills_v2_reset_complete()? {
            return Err(failure(
                "the owning home kernel must complete Skills startup before dispatch",
            ));
        }
        let config = self.config.snapshot();
        Ok(SkillCatalogService::new(SkillCatalogStorage::new(
            config
                .user_config_path
                .parent()
                .ok_or_else(|| failure("resolved Skills home is unavailable"))?,
            self.durable.clone(),
        )?))
    }
    fn resolve_catalog(
        &self,
        service: &SkillCatalogService,
        prompt: &crate::session::PromptQueueItem,
        run: &RuntimeProviderRun,
    ) -> Result<SkillCatalogSet, DaemonError> {
        // Always inspect the durable revision, even if an invalidation was missed.
        let latest = service.storage.snapshot()?;
        if let Some(pinned) = prompt
            .skill_turn_authority()
            .filter(|authority| authority.provider_run_id == run.id())
        {
            // A running turn (including a self-install) keeps its admitted catalog.
            // Queued prompts have no turn authority until this dispatch boundary.
            return if latest.revision == pinned.catalog_revision {
                Ok(latest)
            } else {
                service
                    .storage
                    .snapshot_at_revision(pinned.catalog_revision)
            };
        }
        Ok(latest)
    }
}
impl SkillPromptBridge for SkillDispatchService {
    fn acknowledge_prompt(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        self.record_prompt_receipts(run, session_id, agent_id, prompt_id)
    }
    fn acknowledge_tools(
        &self,
        run: &RuntimeProviderRun,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        self.record_tool_receipts(run, batch)
    }
    fn prepare(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        _prompt_id: &str,
        origin_prompt_id: &str,
        envelope: &PromptEnvelope,
    ) -> Result<Option<ProviderSkillDelivery>, DaemonError> {
        if !super::skills_catalog_runtime::SKILLS_V2_RUNTIME_INTEGRATED {
            return Ok(None);
        }
        if self.runs.is_leased_provider_run(run.id()) {
            return Err(failure("leased skill delivery requires the authenticated home turn binding; no worker catalog fallback is allowed"));
        }
        if run.session_id() != session_id || run.agent_instance_id() != Some(agent_id) {
            return Err(failure(
                "skill provider run does not match its admitted session and agent",
            ));
        }
        let session = self.sessions.get_session(session_id)?;
        let agent = self.agents.get_agent(agent_id)?;
        let prompt = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| prompt.id() == origin_prompt_id)
            .ok_or_else(|| failure("skill dispatch belongs to a different active turn"))?;
        let service = self.catalog()?;
        let catalog = self.resolve_catalog(&service, &prompt, run)?;
        let rows = crate::skill::resolver::projections(&catalog, Some(session.project_id()), &[]);
        let needs_inventory = !rows.is_empty()
            || prompt
                .skill_selections()
                .iter()
                .any(|selection| matches!(selection, SkillSelection::Native { .. }));
        let config = self.config.snapshot();
        let optional_native_syntax = prompt.user_skill_syntax()
            && !crate::skill::selection::explicit_skill_names(prompt.prompt()).is_empty();
        let inventory = if needs_inventory {
            Some(self.inventories.for_run(
                run,
                &session,
                Some(&agent),
                &config,
                session.project_id(),
                catalog.revision,
                false,
            )?)
        } else if optional_native_syntax {
            // A failed discovery query cannot turn unresolved dollar prose into
            // a selected skill. Typed native refs still require the strict path.
            self.inventories
                .for_run(
                    run,
                    &session,
                    Some(&agent),
                    &config,
                    session.project_id(),
                    catalog.revision,
                    false,
                )
                .ok()
        } else {
            None
        };
        let native = inventory
            .as_ref()
            .map(|inventory| inventory.entries.as_slice())
            .unwrap_or_default();
        let usage = run.usage();
        let attachment_bytes = envelope.attachments.iter().fold(0u64, |total, attachment| {
            total.saturating_add(attachment.contents_base64().map(str::len).unwrap_or(0) as u64)
        });
        let remaining = crate::skill::resolver::remaining_turn_budget(
            usage.context_window,
            usage.context_tokens,
            (envelope.hidden_system_context.len() as u64)
                .saturating_add(envelope.visible_user_prompt.len() as u64)
                .saturating_add(attachment_bytes),
        );
        let generation = prompt
            .skill_turn_authority()
            .filter(|authority| authority.provider_run_id == run.id())
            .map(|authority| authority.context_generation.clone())
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}",
                    run.id(),
                    prompt.id(),
                    prompt.durable_recovery_operation_id().unwrap_or("initial")
                )
            });
        let mut context = service.pinned_turn_from_snapshot(
            &catalog,
            Some(session.project_id()),
            agent_id,
            session_id,
            if prompt.user_skill_syntax() {
                prompt.prompt()
            } else {
                ""
            },
            prompt.skill_selections(),
            native,
            usage.context_window,
            config.user_config.skills.metadata_budget_percent.get(),
            remaining,
            &generation,
        )?;
        let same_acknowledged_context = prompt.skill_turn_authority().is_some_and(|authority| {
            authority.provider_run_id == run.id() && authority.context_generation == generation
        }) && prompt.durable_delivery_phase()
            == Some(crate::session::DurablePromptDeliveryPhase::Delivered);
        if envelope.steering && same_acknowledged_context {
            context.metadata.clear();
            context.loaded.retain(|body| {
                !prompt.skill_turn_authority().is_some_and(|authority| {
                    authority.pending_loads.iter().any(|loaded| {
                        loaded.skill_id == body.skill_id && loaded.digest == body.digest
                    })
                })
            });
            if crate::provider::canonical_provider_family(run.provider()) == Some("codex") {
                // The accepted official skill input items already supplied these bodies.
                context.native_selections.clear();
            }
        }
        let delivery = match &inventory {
            Some(inventory) => crate::provider::skill_delivery::prepare_skill_delivery(
                crate::provider::canonical_provider_family(run.provider())
                    .ok_or_else(|| failure("unsupported skill provider"))?,
                &envelope.visible_user_prompt,
                &context,
                inventory,
                prompt.skill_selections(),
            )?,
            None => ProviderSkillDelivery {
                visible_prompt: envelope.visible_user_prompt.clone(),
                hidden_context: context.metadata.clone(),
                codex_input: vec![],
                requires_native_load_ack: vec![],
            },
        };
        let extra_visible = delivery
            .visible_prompt
            .len()
            .saturating_sub(envelope.visible_user_prompt.len()) as u64;
        let mut authority = SkillTurnAuthority {
            catalog_revision: catalog.revision,
            project_id: session.project_id().into(),
            provider_run_id: run.id().into(),
            context_generation: generation.clone(),
            explicit_skill_ids: context
                .loaded
                .iter()
                .filter(|body| body.reason == SkillLoadReason::Explicit)
                .map(|body| body.skill_id.clone())
                .collect(),
            body_budget_tokens: remaining
                .saturating_sub(context.metadata.len() as u64)
                .saturating_sub(extra_visible),
            reserved_body_tokens: BTreeMap::new(),
            pending_loads: vec![],
            prompt_delivery_skill_ids: Default::default(),
        };
        if let Some(previous) = prompt
            .skill_turn_authority()
            .filter(|previous| previous.context_generation == generation)
        {
            authority
                .explicit_skill_ids
                .extend(previous.explicit_skill_ids.iter().cloned());
            authority.explicit_skill_ids.sort();
            authority.explicit_skill_ids.dedup();
            authority.reserved_body_tokens = previous.reserved_body_tokens.clone();
            authority.pending_loads = previous.pending_loads.clone();
            authority.prompt_delivery_skill_ids = previous.prompt_delivery_skill_ids.clone();
        }
        for body in &context.loaded {
            if !matches!(body.delivery, SkillDelivery::ProvidedBy { .. })
                || crate::provider::canonical_provider_family(run.provider()) == Some("codex")
            {
                authority
                    .prompt_delivery_skill_ids
                    .insert(body.skill_id.clone());
            }
            if !matches!(body.delivery, SkillDelivery::ProvidedBy { .. }) {
                let wrapper = format!(
                    "\nSkill {} ({}) at {}:\n{}\n",
                    body.name, body.digest, body.root, body.body
                );
                authority
                    .reserve_body(&body.skill_id, wrapper.len() as u64)
                    .map_err(failure)?;
            }
            if !authority
                .pending_loads
                .iter()
                .any(|pending| pending.skill_id == body.skill_id && pending.digest == body.digest)
            {
                authority.pending_loads.push(SkillLoadRecord {
                    skill_id: body.skill_id.clone(),
                    digest: body.digest.clone(),
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                    run_id: run.id().into(),
                    prompt_id: prompt.id().into(),
                    reason: body.reason.clone(),
                    context_generation: generation.clone(),
                    delivered_at_ms: 0,
                });
            }
        }
        for selection in &context.native_selections {
            let SkillSelection::Native { entry_id, .. } = selection else {
                continue;
            };
            if context.loaded.iter().any(|body|matches!(&body.delivery,SkillDelivery::ProvidedBy{entry_id:id,..} if id==entry_id)) {continue}
            let Some(entry) = native.iter().find(|entry| &entry.selection == selection) else {
                continue;
            };
            let Some(digest) = entry.digest.as_ref().filter(|_| entry.complete_manifest) else {
                continue;
            };
            if !authority
                .pending_loads
                .iter()
                .any(|record| record.skill_id == *entry_id && record.digest == *digest)
            {
                authority.pending_loads.push(SkillLoadRecord {
                    skill_id: entry_id.clone(),
                    digest: digest.clone(),
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                    run_id: run.id().into(),
                    prompt_id: prompt.id().into(),
                    reason: SkillLoadReason::ConfirmedNative,
                    context_generation: generation.clone(),
                    delivered_at_ms: 0,
                });
            }
            if crate::provider::canonical_provider_family(run.provider()) == Some("codex") {
                authority.prompt_delivery_skill_ids.insert(entry_id.clone());
            }
        }
        self.prompts.update_skill_authority_durably(&self.sessions,&self.durable,session_id,agent_id,prompt.id(),|current| {
            if current.prompt()!=prompt.prompt() || current.skill_selections()!=prompt.skill_selections()
                || current.skill_turn_authority()!=prompt.skill_turn_authority() {
                return Err(failure("skill source or turn changed during preparation; retry on the current turn"));
            }
            Ok(authority)
        })?;
        Ok(Some(delivery))
    }
}
