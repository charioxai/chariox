//! MP-08 / MP-10 / MP-11: verify and adapt the home-pinned context on its execution worker.
use super::super::*;
use crate::provider::skill_delivery::TypedSkillLoad;
use crate::skill::leased_composer::{
    ComposerSkillContext, ComposerSkillPrompt, PREPARE_LEASED_SKILL_TURN,
};
use crate::transport::relay_peer::{RemoteSkillSyncContext, RemoteWorkspaceLiveSyncContext};

impl SkillDispatchService {
    pub(in crate::runtime::state) fn prepare_worker_composer(
        &self,
        run: &RuntimeProviderRun,
        authority: &RemoteWorkspaceLiveSyncContext,
        prompt: &crate::session::PromptQueueItem,
        lease_id: &str,
        names: Option<&crate::skill::leased_names::LeasedSkillNames>,
    ) -> Result<ProviderSkillDelivery, DaemonError> {
        let session = self.sessions.get_session(run.session_id())?;
        let agent = run
            .agent_instance_id()
            .map(|id| self.agents.get_agent(id))
            .transpose()?;
        let inventory = self.inventories.for_run(
            run,
            &session,
            agent.as_ref(),
            &self.config.snapshot(),
            0,
            false,
        )?;
        let home = worker_skill_home()?;
        let sync = skill_sync_context(authority);
        // A missing or damaged cache asks for retransmission; it is never delivered.
        let cached_digests = names
            .into_iter()
            .flat_map(|names| &names.skills)
            .filter_map(|skill| {
                crate::skill::transfer::read_cached_skill(
                    &home,
                    &sync,
                    lease_id,
                    &skill.skill_id,
                    &skill.digest,
                )
                .ok()
                .map(|_| skill.digest.clone())
            })
            .collect();
        let known_home_skill =
            names.is_some_and(|names| !names.resolve(prompt.prompt()).is_empty());
        let usage = run.usage();
        let request = ComposerSkillPrompt {
            prompt_id: prompt.id().into(),
            prompt: prompt.prompt().into(),
            native: inventory.entries.clone(),
            cached_digests,
            context_window: usage.context_window,
            context_tokens: usage.context_tokens,
            context_intact: self.context_survives_compaction(
                run.session_id(),
                run.agent_instance_id()
                    .ok_or_else(|| failure("native provider has no agent"))?,
                run.provider(),
                run.account_profile(),
                crate::runtime::state::context_handoff::native_session_id(run),
            )?,
        };
        let response = self.send_home_skill_tool(
            authority.clone(),
            PREPARE_LEASED_SKILL_TURN,
            serde_json::to_value(request)
                .map_err(|_| failure("cannot encode native composer prompt"))?,
        )?;
        let context: Option<ComposerSkillContext> = serde_json::from_value(response.payload)
            .map_err(|_| failure("home returned invalid native composer context"))?;
        let next_names = context.as_ref().map(|turn| {
            turn.names
                .clone()
                .unwrap_or_else(|| crate::skill::leased_names::LeasedSkillNames {
                    catalog_revision: turn.context.catalog_revision,
                    skills: vec![],
                })
        });
        let context = match context {
            Some(context) => materialize_composer_context(authority, lease_id, context)?,
            None if known_home_skill => {
                return Err(failure(
                    "the owning home kernel must complete Skills startup before dispatch",
                ))
            }
            None => native_only_context(prompt.prompt(), &inventory.entries),
        };
        let mut delivery = crate::provider::skill_delivery::prepare_skill_delivery(
            crate::provider::canonical_provider_family(run.provider())
                .ok_or_else(|| failure("unsupported skill provider"))?,
            prompt.prompt(),
            &context,
            &inventory,
            &[],
        )?;
        delivery.leased_names = next_names;
        delivery.typed_leased_loads = receipt_loads(&context, &inventory.entries)?;
        Ok(delivery)
    }
}

fn materialize_composer_context(
    authority: &RemoteWorkspaceLiveSyncContext,
    lease_id: &str,
    mut turn: ComposerSkillContext,
) -> Result<SkillTurnContext, DaemonError> {
    if let Some(names) = &turn.names {
        names.check_transferred(&turn.manifest)?;
    }
    let home = worker_skill_home()?;
    let sync = skill_sync_context(authority);
    let materialized = crate::skill::transfer::materialize_received_packages(
        &home,
        &sync,
        lease_id,
        &turn.manifest,
        &turn.packages,
    )?;
    let roots = materialized
        .iter()
        .map(|entry| (entry.skill_id.clone(), entry.materialized_root.clone()))
        .collect();
    let metadata = crate::skill::resolver::metadata_context_at(
        &turn.auto_rows,
        turn.context_window,
        turn.budget_percent,
        Some(&roots),
    )?;
    turn.context.metadata = metadata.text;
    turn.context.metadata_budget_tokens = metadata.budget_tokens;
    turn.context.metadata_tokens_upper_bound = metadata.tokens_upper_bound;
    turn.context.omitted_auto_count = metadata.omitted_count;
    for body in &mut turn.context.loaded {
        if matches!(body.delivery, SkillDelivery::ProvidedBy { .. }) {
            continue;
        }
        let cached = crate::skill::transfer::read_cached_skill(
            &home,
            &sync,
            lease_id,
            &body.skill_id,
            &body.digest,
        )?;
        body.root = cached.root.display().to_string();
        body.body = cached.body;
    }
    Ok(turn.context)
}

fn native_only_context(prompt: &str, entries: &[NativeSkillEntry]) -> SkillTurnContext {
    let known: Vec<_> = entries
        .iter()
        .filter_map(|entry| match &entry.selection {
            SkillSelection::Native { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let native_selections = crate::skill::selection::explicit_skill_tokens_with_names(prompt,&known).iter()
        .filter_map(|token| entries.iter().find(|entry| entry.exposure == NativeSkillExposure::Confirmed
            && matches!(&entry.selection,SkillSelection::Native {name,..} if name.eq_ignore_ascii_case(&token.name))))
        .map(|entry|entry.selection.clone()).collect();
    SkillTurnContext {
        catalog_revision: 0,
        context_generation: String::new(),
        metadata: String::new(),
        metadata_tokens_upper_bound: 0,
        metadata_budget_tokens: 0,
        omitted_auto_count: 0,
        loaded: vec![],
        native_selections,
    }
}

fn worker_skill_home() -> Result<std::path::PathBuf, DaemonError> {
    crate::config::DaemonConfig::default_user_config_path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| failure("Skills require a resolved Chariox home"))
}
fn skill_sync_context(context: &RemoteWorkspaceLiveSyncContext) -> RemoteSkillSyncContext {
    RemoteSkillSyncContext {
        home_kernel_id: context.home_kernel_id.clone(),
        home_session_id: context.home_session_id.clone(),
        home_agent_id: context.home_agent_id.clone(),
        leased_agent_id: context.leased_agent_id.clone(),
    }
}

fn receipt_loads(
    context: &SkillTurnContext,
    native: &[NativeSkillEntry],
) -> Result<Vec<TypedSkillLoad>, DaemonError> {
    let mut loads: Vec<_> = context
        .loaded
        .iter()
        .map(|body| TypedSkillLoad::Chariox {
            skill_id: body.skill_id.clone(),
            digest: body.digest.clone(),
            reason: body.reason.clone(),
        })
        .collect();
    for selection in &context.native_selections {
        let provided = context.loaded.iter().any(|body| matches!((&body.delivery, selection),
                (SkillDelivery::ProvidedBy {entry_id,..},SkillSelection::Native {entry_id:id,..}) if entry_id == id));
        if !provided {
            let entry = native
                .iter()
                .find(|entry| &entry.selection == selection)
                .filter(|entry| entry.complete_manifest && entry.digest.is_some())
                .ok_or_else(|| failure("native invocation has no verified content receipt"))?;
            loads.push(TypedSkillLoad::Native {
                entry: Box::new(entry.clone()),
            });
        }
    }
    Ok(loads)
}
