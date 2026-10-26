//! MP-08 / MP-10 / MP-11: one instruction preparation service for every structured submission.
use super::*;
#[path = "skills_leased_composer.rs"]
mod composer;
#[path = "skills_leased_delivery_receipts.rs"]
mod leased_receipts;
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
pub(super) use composer::prepare_composer_turn;
pub(super) use leased_receipts::{acknowledge_leased_delivery, LEASED_SKILL_DELIVERY_ACK};

/// Store handles only: this bridge deliberately holds no ProviderProcessServiceStore
/// and cannot form a callback cycle through the provider mailbox.
struct SkillDispatchService {
    changes: Arc<RuntimeChangeSignal>,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    config: crate::runtime::projection::DaemonConfigProjectionStore,
    durable: DurableKernelStateStore,
    sessions: SessionStateStore,
    agents: AgentServiceStore,
    prompts: crate::runtime::prompt_state::PromptStateOwner,
    runs: crate::runtime::projection::ProviderRunProjectionStore,
    inventories: Arc<super::skills_native_inventory::SkillInventories>,
    settled_worker_runs: Arc<super::skills_settled_worker_runs::SettledWorkerRuns>,
    history: crate::history::OperationalHistoryStore,
}
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.dispatch",
        message: message.into(),
    }
}
pub(super) fn bridge(state: &KernelRuntimeState) -> Arc<dyn SkillPromptBridge> {
    Arc::new(service(state))
}
/// Only a selection the client resolved names a skill; `$word` prose that nothing
/// resolved is an ordinary prompt, so a kernel without the catalog still admits it.
fn names_skills(prompt: &crate::session::PromptQueueItem) -> bool {
    !prompt.skill_selections().is_empty()
}
fn service(state: &KernelRuntimeState) -> SkillDispatchService {
    let owned = &state.owned;
    SkillDispatchService {
        changes: owned.runtime_projection_changes.clone(),
        relay_state: owned.relay_state.clone(),
        config: owned.config_projection.clone(),
        durable: owned.durable_state_store.clone(),
        sessions: owned.session_store.clone(),
        agents: owned.agent_store.clone(),
        prompts: owned.prompt_state_owner.clone(),
        runs: owned.provider_run_projection.clone(),
        inventories: owned.skill_inventories.clone(),
        settled_worker_runs: owned.settled_worker_runs.clone(),
        history: owned.operational_history_store.clone(),
    }
}
/// What a leased worker must hold: every effective Auto and Manual package, and the
/// home's names for them. The worker resolves nothing else.
pub(super) struct RemoteSkillSet {
    pub manifest: SkillTransferManifest,
    pub packages: Vec<crate::skill::CharioxSkillPackage>,
    pub names: Option<crate::skill::leased_names::LeasedSkillNames>,
}
/// Home-side delivery for a leased worker: the exact packages the worker must hold
/// before the turn, and the hidden context that names the verified worker roots.
pub(super) struct RemoteSkillTurn {
    pub set: RemoteSkillSet,
    auto_rows: Vec<SkillProjection>,
    loaded: Vec<SkillResolvedBody>,
    budget_percent: f64,
    /// Validated `From <Provider>` selections for the worker's own harness.
    pub native_selections: Vec<SkillSelection>,
    pub catalog_revision: u64,
}
/// The skills a native composer on a leased worker may select before any turn
/// is dispatched from the home; `None` when this kernel does not own the catalog.
pub(super) fn prepare_remote_launch_skills(
    state: &KernelRuntimeState,
    session_id: &str,
) -> Result<Option<RemoteSkillSet>, DaemonError> {
    service(state).prepare_remote_launch(session_id)
}
/// The leased agent's worker provider run as the worker reported it when the home
/// read its native inventory just before the turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReportedWorkerRun {
    /// Not read for this turn, or the worker predates the report.
    Unknown,
    /// The lease has no provider run: the turn starts a new one.
    NoRun,
    /// The worker-local id of the run that takes the turn.
    Live(String),
}
pub(super) fn record_worker_compactions(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    leased_agent_id: &str,
    ids: &[String],
) -> Result<(), DaemonError> {
    service(state).record_worker_compactions(session_id, agent_id, leased_agent_id, ids)
}
pub(super) fn prepare_remote_turn(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
    leased_agent_id: &str,
    native: &[NativeSkillEntry],
    worker_run: &ReportedWorkerRun,
) -> Result<Option<RemoteSkillTurn>, DaemonError> {
    service(state).prepare_remote(
        session_id,
        agent_id,
        prompt_id,
        leased_agent_id,
        native,
        worker_run,
    )
}
/// A leased turn with no effective skill, no native selection and no `$name`
/// syntax gains nothing from the worker's harness inventory: skip the round trip.
pub(super) fn remote_turn_needs_native_inventory(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
) -> Result<bool, DaemonError> {
    service(state).remote_turn_needs_native_inventory(session_id, agent_id, prompt_id)
}
/// A leased turn has no home provider run; its catalog authority is keyed by the lease.
pub(super) fn leased_run_id(leased_agent_id: &str) -> String {
    format!(
        "{}{leased_agent_id}",
        crate::skill::turn_authority::LEASED_RUN_PREFIX
    )
}
/// Pin the worker-verified package roots on the home turn so `load_skill` names paths the
/// leased agent can read.
pub(super) fn bind_remote_roots(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
    materialized: &[crate::transport::relay_peer::RemoteSkillMaterialization],
) -> Result<(), DaemonError> {
    let roots: BTreeMap<String, String> = materialized
        .iter()
        .map(|entry| (entry.skill_id.clone(), entry.materialized_root.clone()))
        .collect();
    state
        .owned
        .update_skill_turn_authority(session_id, agent_id, prompt_id, |authority| {
            authority.remote_roots = roots.clone();
            Ok(())
        })
}
/// Native duplicates retain home load authority, but never transfer a body/package,
/// unless this turn delivers the Chariox copy because the native one cannot acknowledge.
fn remote_skill_set(
    service: &SkillCatalogService,
    catalog: &SkillCatalogSet,
    project_id: &str,
    rows: &[SkillProjection],
    chariox_delivered: &BTreeSet<&str>,
) -> Result<RemoteSkillSet, DaemonError> {
    let mut entries = Vec::new();
    let mut packages = Vec::new();
    for row in rows.iter().filter(|row| {
        row.effective_state != SkillState::Off
            && (!matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
                || chariox_delivered.contains(row.skill.id.as_str()))
    }) {
        let root = service.verified_package_root(&row.skill)?;
        packages.push(crate::skill::package_skill_directory(&root)?);
        entries.push(SkillTransferEntry {
            skill_id: row.skill.id.clone(),
            digest: row.skill.digest.clone(),
            scope: row.skill.scope.clone(),
            state: row.effective_state,
        });
    }
    Ok(RemoteSkillSet {
        manifest: SkillTransferManifest {
            catalog_revision: catalog.revision,
            project_id: Some(project_id.to_string()),
            skills: entries,
        },
        packages: crate::skill::transfer::unique_payloads(packages),
        names: crate::skill::leased_names::LeasedSkillNames::from_projections(
            catalog.revision,
            rows,
        ),
    })
}
impl RemoteSkillTurn {
    pub(super) fn hidden_context(
        &self,
        materialized: &[crate::transport::relay_peer::RemoteSkillMaterialization],
    ) -> Result<String, DaemonError> {
        let roots: BTreeMap<String, String> = materialized
            .iter()
            .map(|entry| (entry.skill_id.clone(), entry.materialized_root.clone()))
            .collect();
        let mut text = crate::skill::resolver::metadata_context_at(
            &self.auto_rows,
            None,
            self.budget_percent,
            Some(&roots),
        )?
        .text;
        for body in &self.loaded {
            let root = roots
                .get(&body.skill_id)
                .ok_or_else(|| failure("worker receipt is missing a selected skill"))?;
            text.push_str(&format!(
                "\nSkill {} ({}) at {}:\n{}\n",
                body.name, body.digest, root, body.body
            ));
            text.extend(crate::skill::resolver::conflict_notice(body));
        }
        Ok(text)
    }
}
impl SkillDispatchService {
    fn prepare_remote_launch(
        &self,
        session_id: &str,
    ) -> Result<Option<RemoteSkillSet>, DaemonError> {
        if !self.catalog_available() {
            return Ok(None);
        }
        let service = self.catalog()?;
        let catalog = service.storage.snapshot()?;
        let session = self.sessions.get_session(session_id)?;
        let rows = crate::skill::resolver::projections(&catalog, Some(session.project_id()), &[]);
        let set = remote_skill_set(
            &service,
            &catalog,
            session.project_id(),
            &rows,
            &BTreeSet::new(),
        )?;
        // An empty set is sent too: it is how a lease forgets skills turned Off or removed.
        Ok(Some(set))
    }
    fn remote_turn_needs_native_inventory(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| prompt.id() == prompt_id)
            .ok_or_else(|| failure("skill dispatch belongs to a different active turn"))?;
        if prompt
            .skill_selections()
            .iter()
            .any(|selection| matches!(selection, SkillSelection::Native { .. }))
        {
            return Ok(true);
        }
        // Without the catalog nothing can resolve `$word` prose: it is an ordinary prompt.
        if !self.catalog_available() {
            return Ok(false);
        }
        if prompt.user_skill_syntax()
            && !crate::skill::selection::explicit_skill_names(prompt.prompt()).is_empty()
        {
            return Ok(true);
        }
        let catalog = self.catalog()?.storage.snapshot()?;
        Ok(
            crate::skill::resolver::projections(&catalog, Some(session.project_id()), &[])
                .iter()
                .any(|row| row.effective_state != SkillState::Off),
        )
    }
    /// The run whose provider context still holds earlier skill loads. A compaction or
    /// profile switch waiting for its recovery turn means that context is gone, so
    /// every active load must be delivered again.
    fn live_context_run_id<'a>(
        &self,
        session_id: &str,
        agent_id: &str,
        run: &'a RuntimeProviderRun,
    ) -> Result<Option<&'a str>, DaemonError> {
        let intact = self.context_survives_compaction(
            session_id,
            agent_id,
            run.provider(),
            run.account_profile(),
            super::context_handoff::native_session_id(run),
        )?;
        Ok(intact.then(|| run.id()))
    }
    fn context_survives_compaction(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        account: &str,
        native_session: Option<&str>,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .history
            .load_agent_compaction_boundary(
                session_id,
                agent_id,
                provider,
                account,
                native_session,
            )?
            .is_none())
    }
    /// The projected id of the worker provider run that still holds this agent's earlier
    /// loads. The home learns a run when a turn settles on it, so the last settled run is
    /// the run the next turn is assumed to use. A home projection of another run means the
    /// home cannot say which run that is, and a worker that reports another run (or none)
    /// at turn start relaunched unseen: nothing is assumed held.
    fn live_leased_run_id(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
        worker_run: &ReportedWorkerRun,
    ) -> Result<Option<String>, DaemonError> {
        let projected = self.projected_leased_run(session_id, agent_id, leased_agent_id);
        let settled = self.settled_worker_runs.last(agent_id, leased_agent_id);
        let live = match (&projected, settled) {
            (Some(run), Some(settled)) if run.id() != settled => return Ok(None),
            (Some(run), _) => run.id().to_string(),
            (None, Some(settled)) => settled,
            (None, None) => return Ok(None),
        };
        match worker_run {
            ReportedWorkerRun::Unknown => {}
            ReportedWorkerRun::NoRun => return Ok(None),
            ReportedWorkerRun::Live(run) => {
                if crate::provider::projected_leased_provider_run_id(leased_agent_id, run) != live {
                    return Ok(None);
                }
            }
        }
        let intact = self
            .pending_leased_compaction(session_id, agent_id, leased_agent_id)?
            .is_none();
        Ok(intact.then_some(live))
    }
    fn projected_leased_run(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs.get_for_agent(session_id, agent_id).filter(|run| {
            crate::provider::worker_provider_run_id_from_projected_leased_id(
                leased_agent_id,
                run.id(),
            )
            .is_some()
        })
    }
    /// A leased agent's recovery epochs live under the home's view of its native
    /// session: the projected worker run, else what the agent last resumed.
    fn leased_context_identity(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
    ) -> Result<(String, String, String, Option<String>), DaemonError> {
        Ok(
            match self.projected_leased_run(session_id, agent_id, leased_agent_id) {
                Some(run) => (
                    run.provider().to_string(),
                    run.model().to_string(),
                    run.account_profile().to_string(),
                    super::context_handoff::native_session_id(&run).map(str::to_string),
                ),
                None => {
                    let agent = self.agents.get_agent(agent_id)?;
                    (
                        agent.provider().to_string(),
                        agent.model().unwrap_or_default().to_string(),
                        agent.provider_account_profile().to_string(),
                        agent
                            .provider_resume_state()
                            .provider_session_id(agent.provider())
                            .map(str::to_string),
                    )
                }
            },
        )
    }
    fn pending_leased_compaction(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
    ) -> Result<Option<u64>, DaemonError> {
        let (provider, _, account, native) =
            self.leased_context_identity(session_id, agent_id, leased_agent_id)?;
        Ok(self
            .history
            .load_agent_compaction_boundary(
                session_id,
                agent_id,
                &provider,
                &account,
                native.as_deref(),
            )?
            .map(|boundary| boundary.revision))
    }
    /// The worker owns its harness, so it reports compactions; each new ID starts a
    /// recovery epoch on the home, where `live_leased_run_id` reads it.
    fn record_worker_compactions(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
        ids: &[String],
    ) -> Result<(), DaemonError> {
        if ids.is_empty() {
            return Ok(());
        }
        let (provider, model, account, native) =
            self.leased_context_identity(session_id, agent_id, leased_agent_id)?;
        for id in ids {
            self.history.record_observed_agent_compaction_boundary(
                session_id,
                agent_id,
                crate::history::AgentCompactionOrigin {
                    provider: &provider,
                    model: &model,
                    account: &account,
                    native_session: native.as_deref(),
                },
                id,
            )?;
        }
        Ok(())
    }
    fn prepare_remote(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        leased_agent_id: &str,
        native: &[NativeSkillEntry],
        worker_run: &ReportedWorkerRun,
    ) -> Result<Option<RemoteSkillTurn>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| prompt.id() == prompt_id)
            .ok_or_else(|| failure("skill dispatch belongs to a different active turn"))?;
        if !self.catalog_available() && !names_skills(&prompt) {
            return Ok(None);
        }
        let service = self.catalog()?;
        // Dispatch always reads the durable revision, so a changed body is picked up next turn.
        let catalog = service.storage.snapshot()?;
        let project_id = session.project_id();
        let rows = crate::skill::resolver::projections(&catalog, Some(project_id), native);
        let budget_percent = self
            .config
            .snapshot()
            .user_config
            .skills
            .metadata_budget_percent
            .get();
        let remaining =
            crate::skill::resolver::remaining_turn_budget(None, None, prompt.prompt().len() as u64);
        let generation = format!("{}:{}:remote", session_id, prompt.id());
        // The worker's current provider run still holds what it was given earlier;
        // a new run, a lease recreation or a compaction re-attaches the loads.
        let live_run_id =
            self.live_leased_run_id(session_id, agent_id, leased_agent_id, worker_run)?;
        let context = service.pinned_turn_from_snapshot(
            &catalog,
            Some(project_id),
            agent_id,
            session_id,
            if prompt.user_skill_syntax() {
                prompt.prompt()
            } else {
                ""
            },
            prompt.skill_selections(),
            native,
            None,
            budget_percent,
            remaining,
            &generation,
            live_run_id.as_deref(),
        )?;
        // Native duplicates retain home load authority, but never transfer a body/package.
        let auto_rows: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.effective_state == SkillState::Auto
                    && !matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
            })
            .cloned()
            .collect();
        let chariox_delivered = context
            .loaded
            .iter()
            .filter(|body| !matches!(body.delivery, SkillDelivery::ProvidedBy { .. }))
            .map(|body| body.skill_id.as_str())
            .collect();
        let set = remote_skill_set(&service, &catalog, project_id, &rows, &chariox_delivered)?;
        let run_id = leased_run_id(leased_agent_id);
        let compaction_revision =
            self.pending_leased_compaction(session_id, agent_id, leased_agent_id)?;
        let mut authority = SkillTurnAuthority {
            catalog_revision: catalog.revision,
            project_id: project_id.into(),
            provider_run_id: run_id.clone(),
            context_generation: generation.clone(),
            explicit_skill_ids: context
                .loaded
                .iter()
                .filter(|body| body.reason == SkillLoadReason::Explicit)
                .map(|body| body.skill_id.clone())
                .collect(),
            body_budget_tokens: remaining.saturating_sub(context.metadata.len() as u64),
            reserved_body_tokens: BTreeMap::new(),
            pending_loads: vec![],
            prompt_delivery_skill_ids: Default::default(),
            remote_roots: Default::default(),
            native_tool_loads: Default::default(),
            compaction_revision,
        };
        // The worker reports a completed OpenCode `skill` tool by name; the home maps it.
        // Other harnesses never report one, so the map is inert for them.
        authority.native_tool_loads =
            native_tool_loads(&context.loaded, &context.native_selections, native);
        for body in &context.loaded {
            if !matches!(body.delivery, SkillDelivery::ProvidedBy { .. })
                || matches!(&body.delivery, SkillDelivery::ProvidedBy { provider, .. } if provider == "codex")
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
            authority.pending_loads.push(SkillLoadRecord {
                skill_id: body.skill_id.clone(),
                digest: body.digest.clone(),
                session_id: session_id.into(),
                agent_id: agent_id.into(),
                run_id: run_id.clone(),
                prompt_id: prompt.id().into(),
                reason: body.reason.clone(),
                context_generation: generation.clone(),
                delivered_at_ms: 0,
            });
        }
        // A native-only selection has no Chariox row; the home still records the
        // worker's acknowledged delivery, without any native package copy.
        for (entry_id, digest, provider) in
            verified_native_only(&context.loaded, &context.native_selections, native)
        {
            authority.pending_loads.push(SkillLoadRecord {
                skill_id: entry_id.clone(),
                digest,
                session_id: session_id.into(),
                agent_id: agent_id.into(),
                run_id: run_id.clone(),
                prompt_id: prompt.id().into(),
                reason: SkillLoadReason::ConfirmedNative,
                context_generation: generation.clone(),
                delivered_at_ms: 0,
            });
            if matches!(
                crate::provider::canonical_provider_family(&provider),
                Some("codex" | "claude")
            ) {
                authority.prompt_delivery_skill_ids.insert(entry_id);
            }
        }
        // Dispatch retries rebuild the same turn: keep the first admitted authority
        // (including loads already reserved by a running list/load tool).
        if prompt
            .skill_turn_authority()
            .is_none_or(|current| current.context_generation != generation)
        {
            self.prompts.update_skill_authority_durably(
                &self.sessions,
                &self.durable,
                session_id,
                agent_id,
                prompt.id(),
                |_| Ok(authority.clone()),
            )?;
        }
        Ok(Some(RemoteSkillTurn {
            set,
            auto_rows,
            loaded: context
                .loaded
                .into_iter()
                .filter(|body| !matches!(body.delivery, SkillDelivery::ProvidedBy { .. }))
                .collect(),
            budget_percent,
            native_selections: context.native_selections,
            catalog_revision: catalog.revision,
        }))
    }
}
impl SkillDispatchService {
    /// The execution kernel applies its own harness invocation to the selections the
    /// home validated; it resolves nothing and no native package is copied.
    fn prepare_leased_native(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        origin_prompt_id: &str,
        envelope: &PromptEnvelope,
    ) -> Result<Option<ProviderSkillDelivery>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| prompt.id() == origin_prompt_id)
            .ok_or_else(|| failure("leased skill prompt is no longer active"))?;
        if prompt.durable_delivery_phase()
            == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            && !crate::provider::provider_run_uses_claude_native_bridge(run)
        {
            return Ok(None);
        }
        // Claude PTY admission can precede its first injection while startup
        // settles. Revalidate/rewrite the pinned invocation at that boundary.
        let staged = prompt.leased_native_skills();
        if staged.selections.is_empty() {
            // A nonzero home catalog revision binds this leased skill context.
            // Its body is already in the envelope: preserve strict ACK semantics
            // without appending it a second time or discovering a worker catalog.
            if prompt.leased_skill_context().is_some()
                && staged.catalog_revision > 0
                && !envelope.hidden_system_context.is_empty()
            {
                return Ok(Some(ProviderSkillDelivery {
                    visible_prompt: envelope.visible_user_prompt.clone(),
                    hidden_context: String::new(),
                    codex_input: vec![],
                    requires_native_load_ack: vec![],
                    typed_leased_loads: vec![],
                    leased_names: None,
                }));
            }
            return Ok(None);
        }
        let session = self.sessions.get_session(session_id)?;
        let agent = self.agents.get_agent(agent_id)?;
        let inventory = self.inventories.for_run(
            run,
            &session,
            Some(&agent),
            &self.config.snapshot(),
            staged.catalog_revision,
            false,
        )?;
        let context = SkillTurnContext {
            catalog_revision: staged.catalog_revision,
            context_generation: format!("{}:{origin_prompt_id}:native", run.id()),
            metadata: String::new(),
            metadata_tokens_upper_bound: 0,
            metadata_budget_tokens: 0,
            omitted_auto_count: 0,
            loaded: Vec::new(),
            native_selections: staged.selections,
        };
        crate::provider::skill_delivery::prepare_skill_delivery(
            crate::provider::canonical_provider_family(run.provider())
                .ok_or_else(|| failure("unsupported skill provider"))?,
            &envelope.visible_user_prompt,
            &context,
            &inventory,
            &[],
        )
        .map(Some)
    }
    /// A second kernel sharing the Chariox home does not own the catalog: its ordinary
    /// turns carry no skills, while a turn that names one is refused by `catalog`.
    fn catalog_available(&self) -> bool {
        super::skills_catalog_runtime::catalog_available(&self.durable)
    }
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
    fn acknowledge_hook_context(
        &self,
        run: &RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        self.record_hook_receipts(run, session_id, agent_id, prompt_id)
    }
    fn acknowledge_tools(
        &self,
        run: &RuntimeProviderRun,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        self.record_tool_receipts(run, batch)
    }
    fn prepare_typed_leased(
        &self,
        run: &RuntimeProviderRun,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        prompt: &crate::session::PromptQueueItem,
        lease_id: &str,
        names: Option<&crate::skill::leased_names::LeasedSkillNames>,
    ) -> Result<ProviderSkillDelivery, DaemonError> {
        self.prepare_worker_composer(run, context, prompt, lease_id, names)
    }
    fn acknowledge_typed_leased(
        &self,
        _run: &RuntimeProviderRun,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        prompt_id: &str,
        skills: &[crate::provider::skill_delivery::TypedSkillLoad],
    ) -> Result<(), DaemonError> {
        self.forward_typed_delivery(context, prompt_id, skills)
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
        // A leased worker holds no catalog: the authenticated home builds the delivery
        // from its own revision and submits it as hidden context (see prepare_remote_turn).
        let session = self.sessions.get_session(session_id)?;
        let leased = self
            .prompts
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| {
                prompt.leased_skill_context().is_some()
                    || !prompt.leased_native_skills().is_unbound()
            });
        if self.runs.is_leased_provider_run(run.id()) || leased {
            return self.prepare_leased_native(
                run,
                session_id,
                agent_id,
                origin_prompt_id,
                envelope,
            );
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
        if !self.catalog_available() && !names_skills(&prompt) {
            return Ok(None);
        }
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
            self.live_context_run_id(session_id, agent_id, run)?,
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
                typed_leased_loads: vec![],
                leased_names: None,
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
            remote_roots: Default::default(),
            native_tool_loads: Default::default(),
            compaction_revision: None,
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
        let opencode =
            crate::provider::canonical_provider_family(run.provider()) == Some("opencode");
        if opencode {
            authority.native_tool_loads =
                native_tool_loads(&context.loaded, &context.native_selections, native);
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
        for (entry_id, digest, _) in
            verified_native_only(&context.loaded, &context.native_selections, native)
        {
            if !authority
                .pending_loads
                .iter()
                .any(|record| record.skill_id == entry_id && record.digest == digest)
            {
                authority.pending_loads.push(SkillLoadRecord {
                    skill_id: entry_id.clone(),
                    digest,
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                    run_id: run.id().into(),
                    prompt_id: prompt.id().into(),
                    reason: SkillLoadReason::ConfirmedNative,
                    context_generation: generation.clone(),
                    delivered_at_ms: 0,
                });
            }
            // Codex acknowledges the skill input item; Claude runs the selection as the
            // `/name` command of the submitted prompt, which its prompt hook receipt confirms.
            if matches!(
                crate::provider::canonical_provider_family(run.provider()),
                Some("codex" | "claude")
            ) {
                authority.prompt_delivery_skill_ids.insert(entry_id);
            }
        }
        self.prompts.update_skill_authority_durably(&self.sessions,&self.durable,session_id,agent_id,prompt.id(),|current| {
            if current.prompt()!=prompt.prompt() || current.skill_selections()!=prompt.skill_selections()
                || current.skill_turn_authority()!=prompt.skill_turn_authority() {
                return Err(failure("skill source or turn changed during preparation; retry on the current turn"));
            }
            Ok(authority)
        })?;
        // A turn with no effective skill keeps the provider's ordinary path.
        let carries_nothing = delivery.hidden_context.is_empty()
            && delivery.codex_input.is_empty()
            && delivery.requires_native_load_ack.is_empty()
            && delivery.visible_prompt == envelope.visible_user_prompt;
        Ok((!carries_nothing).then_some(delivery))
    }
}

/// `From <Provider>` selections the harness runs itself with a complete verified
/// manifest, and no Chariox row provides: the (entry ID, digest, provider) each settles as.
fn verified_native_only(
    loaded: &[SkillResolvedBody],
    selections: &[SkillSelection],
    native: &[NativeSkillEntry],
) -> Vec<(String, String, String)> {
    selections
        .iter()
        .filter_map(|selection| {
            let SkillSelection::Native {
                entry_id, provider, ..
            } = selection
            else {
                return None;
            };
            let provided = loaded.iter().any(|body| {
                matches!(&body.delivery, SkillDelivery::ProvidedBy { entry_id: id, .. } if id == entry_id)
            });
            if provided {
                return None;
            }
            let entry = native.iter().find(|entry| &entry.selection == selection)?;
            let digest = entry.digest.as_ref().filter(|_| entry.complete_manifest)?;
            Some((entry_id.clone(), digest.clone(), provider.clone()))
        })
        .collect()
}

/// OpenCode loads deduped Chariox skills and `From OpenCode` selections with its own
/// `skill` tool, by name. Map each name to the pending load that tool call settles: the
/// Chariox skill a native copy provides, or the selected native entry.
fn native_tool_loads(
    loaded: &[SkillResolvedBody],
    selections: &[SkillSelection],
    native: &[NativeSkillEntry],
) -> BTreeMap<String, String> {
    let mut loads: BTreeMap<String, String> = loaded
        .iter()
        .filter(|body| matches!(body.delivery, SkillDelivery::ProvidedBy { .. }))
        .map(|body| (body.name.clone(), body.skill_id.clone()))
        .collect();
    for selection in selections {
        let SkillSelection::Native { entry_id, name, .. } = selection else {
            continue;
        };
        let provided = loaded.iter().any(|body| {
            matches!(&body.delivery, SkillDelivery::ProvidedBy { entry_id: id, .. } if id == entry_id)
        });
        let verified = native.iter().any(|entry| {
            &entry.selection == selection && entry.complete_manifest && entry.digest.is_some()
        });
        if !provided && verified {
            loads.insert(name.clone(), entry_id.clone());
        }
    }
    loads
}

#[cfg(test)]
mod native_tool_load_tests {
    use super::*;

    fn native(name: &str) -> NativeSkillEntry {
        NativeSkillEntry {
            selection: SkillSelection::Native {
                entry_id: format!("entry-{name}"),
                name: name.into(),
                provider: "opencode".into(),
                account_profile: "default".into(),
                execution_id: "kernel:run".into(),
                inventory_revision: "inventory".into(),
            },
            description: String::new(),
            digest: Some("a".repeat(64)),
            exposure: crate::skill::v2::NativeSkillExposure::Confirmed,
            default_state: SkillState::Auto,
            complete_manifest: true,
            supports_load_ack: true,
            discovery_error: None,
        }
    }

    // A deduped `$name` resolves to the native entry too; its tool call must still
    // settle the Chariox skill's pending load, not the native entry.
    #[test]
    fn a_deduped_skill_maps_to_its_chariox_load_and_a_native_selection_to_its_entry() {
        let dup = native("dup");
        let only = native("only");
        let provided = SkillResolvedBody {
            skill_id: "skill-dup".into(),
            name: "dup".into(),
            digest: "a".repeat(64),
            root: String::new(),
            body: String::new(),
            files: vec![],
            reason: SkillLoadReason::Explicit,
            delivery: SkillDelivery::ProvidedBy {
                provider: "opencode".into(),
                entry_id: "entry-dup".into(),
            },
        };
        let loads = native_tool_loads(
            &[provided],
            &[dup.selection.clone(), only.selection.clone()],
            &[dup, only],
        );
        assert_eq!(
            loads,
            BTreeMap::from([
                ("dup".to_string(), "skill-dup".to_string()),
                ("only".to_string(), "entry-only".to_string()),
            ])
        );
    }
}
