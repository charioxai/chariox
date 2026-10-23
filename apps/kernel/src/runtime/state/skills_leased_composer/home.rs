//! MP-08 / MP-10 / MP-11: typed leased turns use the same home resolver as dispatched turns.
use super::super::*;
use crate::skill::leased_composer::{ComposerSkillContext, ComposerSkillPrompt};
use crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext;

pub(in crate::runtime::state) fn prepare_composer_turn(
    state: &KernelRuntimeState,
    context: &RemoteWorkspaceLiveSyncContext,
    arguments: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    state.authorize_forwarded_native_composer_context(context)?;
    let request: ComposerSkillPrompt = serde_json::from_value(arguments)
        .map_err(|_| failure("invalid native composer skill request"))?;
    let prepared = service(state).home_composer_context(context, request)?;
    Ok(crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::to_value(prepared)
            .map_err(|_| failure("cannot encode native composer skills"))?,
    })
}

impl SkillDispatchService {
    fn home_composer_context(
        &self,
        authority: &RemoteWorkspaceLiveSyncContext,
        mut request: ComposerSkillPrompt,
    ) -> Result<Option<ComposerSkillContext>, DaemonError> {
        if !self.catalog_available() {
            return Ok(None);
        }
        let service = self.catalog()?;
        let catalog = service.storage.snapshot()?;
        let session = self.sessions.get_session(&authority.home_session_id)?;
        let agent = self.agents.get_agent(&authority.home_agent_id)?;
        let provider = crate::provider::canonical_provider_family(agent.provider());
        let native_execution = format!(
            "{}:{}",
            authority.worker_kernel_id, authority.worker_provider_run_id
        );
        if request.native.iter().any(|entry| !matches!(&entry.selection,
            SkillSelection::Native {provider: actual, execution_id, ..} if Some(actual.as_str()) == provider && execution_id == &native_execution)) {
            return Err(failure("native inventory belongs to a different provider or execution; refresh Skills"));
        }
        // This operation is the native composer's hook/receipt path. A verified
        // directory entry can use that acknowledgement even though discovery alone cannot.
        for entry in &mut request.native {
            if entry.exposure == NativeSkillExposure::Confirmed && entry.complete_manifest {
                entry.supports_load_ack = true;
            }
        }
        let budget_percent = self
            .config
            .snapshot()
            .user_config
            .skills
            .metadata_budget_percent
            .get();
        let generation = format!("{}:{}:typed", authority.home_session_id, request.prompt_id);
        let mut live = self.live_leased_run_id(
            session.id(),
            agent.id(),
            &authority.leased_agent_id,
            &ReportedWorkerRun::Live(authority.worker_provider_run_id.clone()),
        )?;
        if !request.context_intact {
            live = None;
        }
        let mut context = service.pinned_turn_from_snapshot(
            &catalog,
            Some(session.project_id()),
            agent.id(),
            session.id(),
            &request.prompt,
            &[],
            &request.native,
            request.context_window,
            budget_percent,
            crate::skill::resolver::remaining_turn_budget(
                request.context_window,
                request.context_tokens,
                request.prompt.len() as u64,
            ),
            &generation,
            live.as_deref(),
        )?;
        let rows = crate::skill::resolver::projections(
            &catalog,
            Some(session.project_id()),
            &request.native,
        );
        let delivered = context
            .loaded
            .iter()
            .filter(|body| !matches!(body.delivery, SkillDelivery::ProvidedBy { .. }))
            .map(|body| body.skill_id.as_str())
            .collect();
        let mut set =
            remote_skill_set(&service, &catalog, session.project_id(), &rows, &delivered)?;
        set.packages
            .retain(|package| !request.cached_digests.contains(&package.version_hash));
        // Only the transferred immutable package supplies a worker's body and root.
        context.metadata.clear();
        for body in &mut context.loaded {
            body.body.clear();
            body.root.clear();
        }
        Ok(Some(ComposerSkillContext {
            manifest: set.manifest,
            packages: set.packages,
            names: set.names,
            context,
            auto_rows: rows
                .into_iter()
                .filter(|row| {
                    row.effective_state == SkillState::Auto
                        && !matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
                })
                .collect(),
            budget_percent,
            context_window: request.context_window,
        }))
    }
}
