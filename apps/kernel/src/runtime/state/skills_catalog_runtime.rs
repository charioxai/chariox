//! MP-08 / MP-10 / MP-11: home catalog operations shared by clients and runtime tools.
//! Startup admission remains closed until the complete durable reset is certified.
use super::skills_turn_authority::SkillToolOrigin;
use super::*;
use crate::skill::{
    resolver::{admit_load, discovery_page},
    service::SkillCatalogService,
    storage::SkillCatalogStorage,
    v2::*,
};
use base64::Engine;

/// MP-08 / MP-10 / MP-11: only the kernel that owns the home catalog and finished its
/// startup reset serves Skills.
pub(super) fn catalog_available(durable: &DurableKernelStateStore) -> bool {
    durable.owns_skill_catalog() && durable.skills_v2_reset_complete().unwrap_or(false)
}

/// MP-08 / MP-10 / MP-11: all status/registration paths project one kernel-owned gate.
pub(crate) fn control_capabilities(durable: &DurableKernelStateStore) -> Vec<String> {
    capabilities_for_readiness(catalog_available(durable))
}
fn capabilities_for_readiness(ready: bool) -> Vec<String> {
    crate::local::RUNTIME_CONTROL_CAPABILITIES
        .iter()
        .copied()
        .chain(
            ready
                .then_some([
                    crate::local::SKILLS_V2_RUNTIME_CAPABILITY,
                    crate::local::SKILLS_REPOSITORY_UPDATE_CAPABILITY,
                ])
                .into_iter()
                .flatten(),
        )
        .map(str::to_owned)
        .collect()
}

fn unavailable() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2",
        message: "Skills v2 catalog service is not available on this kernel: the home catalog belongs to another kernel or its startup reset is incomplete"
            .into(),
    }
}
fn skill_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    pub(crate) fn with_skills_catalog_caller(
        &self,
        command: &crate::runtime::command::KernelCommand,
    ) -> Self {
        let mut state = self.clone();
        state.skills_catalog_caller = Some((command.source.clone(), command.caller.clone()));
        state
    }

    pub(crate) fn skills_v2_runtime_available(&self) -> bool {
        catalog_available(&self.owned.durable_state_store)
    }
    pub(crate) fn control_capabilities(&self) -> Vec<String> {
        control_capabilities(&self.owned.durable_state_store)
    }
    pub(crate) fn ensure_skills_v2_catalog_ready(&self) -> Result<(), DaemonError> {
        self.skills_catalog_service().map(|_| ())
    }

    pub(super) fn skills_catalog_service(&self) -> Result<SkillCatalogService, DaemonError> {
        if !catalog_available(&self.owned.durable_state_store) {
            return Err(unavailable());
        }
        let config = self.owned.config_projection.snapshot();
        let home = config.user_config_path.parent().ok_or_else(unavailable)?;
        Ok(SkillCatalogService::new(SkillCatalogStorage::new(
            home,
            self.owned.durable_state_store.clone(),
        )?))
    }

    /// The router supplies authenticated caller identity; a JSON skill source
    /// cannot choose its owner, a Project it cannot view, or its mutation policy.
    pub(crate) async fn execute_skills_catalog_request(
        &self,
        request: SkillsV2Request,
        caller_user_id: String,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.execute_skills_catalog_request_with_origin(request, caller_user_id, None)
            .await
    }
    pub(super) async fn execute_skills_catalog_request_with_origin(
        &self,
        request: SkillsV2Request,
        caller_user_id: String,
        origin: Option<SkillToolOrigin>,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let state = self.clone();
        let response = tokio::task::spawn_blocking(move || {
            let result = state.execute_skills_catalog_request_blocking(
                request,
                &caller_user_id,
                origin.as_ref(),
            )?;
            Ok(crate::local::LocalDaemonResponse::SkillsV2 { result })
        })
        .await
        .map_err(|_| skill_error("Skills catalog operation did not complete"))?;
        if matches!(
            response,
            Ok(crate::local::LocalDaemonResponse::SkillsV2 {
                result: SkillsV2Response::Published { .. }
            })
        ) {
            let state = self.clone();
            tokio::spawn(async move {
                state.refresh_leased_native_skill_views().await;
            });
        }
        response
    }

    pub(super) fn skills_host_owner(&self, user: &str) -> bool {
        let config = self.owned.config_projection.snapshot();
        if let Some((source, caller)) = &self.skills_catalog_caller {
            return caller
                .user_id
                .as_deref()
                .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID)
                == user
                && crate::runtime::cloud_relay_authorization::kernel_cloud_caller_owner(
                    &config, source, caller,
                );
        }
        // Internal skill tools and feeds supply an owner from kernel-owned agent
        // or attachment state. Client requests must retain their transport identity.
        user == crate::session::DEFAULT_LOCAL_USER_ID
            || config
                .cloud_relay
                .as_ref()
                .is_some_and(|profile| profile.user_id == user)
    }
    fn ensure_skill_project_visible(
        &self,
        project: Option<&str>,
        user: &str,
    ) -> Result<(), DaemonError> {
        if let Some(id) = project {
            crate::skill::catalog::validate_id(id)?;
            if !self.skills_host_owner(user)
                && !self
                    .owned
                    .session_store
                    .list_visible_projects(user, true)
                    .iter()
                    .any(|project| project.id() == id)
            {
                return Err(skill_error(
                    "Project skills require membership in that Project",
                ));
            }
            self.owned.session_store.get_project(id)?;
        }
        Ok(())
    }
    pub(super) fn ensure_skill_mutation_owner(
        &self,
        mutation: &SkillCatalogMutation,
        user: &str,
    ) -> Result<(), DaemonError> {
        if !self.skills_host_owner(user) {
            return Err(skill_error(
                "the home kernel owner must authorize skill catalog mutations",
            ));
        }
        if let SkillScope::Project { project_id } = &mutation.scope {
            self.ensure_skill_project_visible(Some(project_id), user)?;
        }
        for action in &mutation.actions {
            if let SkillCatalogAction::Move {
                destination: SkillScope::Project { project_id },
                ..
            } = action
            {
                self.ensure_skill_project_visible(Some(project_id), user)?;
            }
        }
        Ok(())
    }
    fn native_review_sources(
        &self,
        mutation: &SkillCatalogMutation,
        user: &str,
        origin: Option<&SkillToolOrigin>,
    ) -> Result<BTreeMap<usize, crate::skill::service::VerifiedNativeSkillSource>, DaemonError>
    {
        let config = self.owned.config_projection.snapshot();
        let prefix = format!("{}:", config.daemon_id);
        let mut sources = BTreeMap::new();
        for (index, action) in mutation.actions.iter().enumerate() {
            let source = match action {
                SkillCatalogAction::Install { source, .. }
                | SkillCatalogAction::Update { source, .. } => source,
                _ => continue,
            };
            let SkillInstallSource::Native { selection } = source else {
                continue;
            };
            let SkillSelection::Native { execution_id, .. } = selection else {
                return Err(skill_error(
                    "Import requires a source-bound From Provider entry",
                ));
            };
            let run_id = execution_id.strip_prefix(&prefix).ok_or_else(|| {
                skill_error("native import must be resolved on the selected execution kernel")
            })?;
            let run = self.owned.provider_store.get_run(run_id)?;
            let agent_id = run
                .agent_instance_id()
                .ok_or_else(|| skill_error("native import requires the focused agent"))?;
            if origin.is_some_and(|origin| {
                origin.agent_id != agent_id || origin.session_id != run.session_id()
            }) {
                return Err(skill_error("agents may import only their own execution-local provider skill; use the user Skills screen for another agent"));
            }
            self.skill_agent_session(run.session_id(), agent_id, user)?;
            let started = std::time::Instant::now();
            let inventory =
                self.local_skill_inventory(agent_id, mutation.expected_revision, false)?;
            let details = inventory.details(selection)?;
            crate::logging::info_with_fields(
                "daemon.skills",
                "skills native import source resolved",
                serde_json::json!({
                    "agent_id": agent_id,
                    "elapsed_ms": started.elapsed().as_millis() as u64,
                }),
            );
            let digest = details.manifest_digest.ok_or_else(|| {
                skill_error("native source files are unavailable; refresh Details before importing")
            })?;
            let root = details
                .root
                .ok_or_else(|| skill_error("native source root is unavailable"))?;
            sources.insert(
                index,
                crate::skill::service::VerifiedNativeSkillSource {
                    selection: selection.clone(),
                    root,
                    digest,
                    source_machine: Some(config.host_machine_id.clone()),
                },
            );
        }
        Ok(sources)
    }

    fn skill_agent_session(
        &self,
        session_id: &str,
        agent_id: &str,
        user: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(skill_error(
                "skill target agent does not belong to this session",
            ));
        }
        let session = self.owned.session_store.get_session(session_id)?;
        if !self.skills_host_owner(user) && !session.has_member(user) {
            return Err(skill_error(
                "session membership is required to view or load its skills",
            ));
        }
        Ok(session)
    }

    fn execute_skills_catalog_request_blocking(
        &self,
        request: SkillsV2Request,
        user: &str,
        origin: Option<&SkillToolOrigin>,
    ) -> Result<SkillsV2Response, DaemonError> {
        if let Some(origin) = origin {
            self.current_skill_tool_prompt(
                &origin.session_id,
                &origin.agent_id,
                &origin.prompt_id,
            )?;
        }
        let service = self.skills_catalog_service()?;
        match request {
            SkillsV2Request::Catalog {
                project_id,
                agent_id,
            } => {
                self.ensure_skill_project_visible(project_id.as_deref(), user)?;
                if let Some(id) = &agent_id {
                    let agent = self.owned.agent_store.get_agent(id)?;
                    let session = self.skill_agent_session(agent.session_id(), id, user)?;
                    if project_id.as_deref() != Some(session.project_id()) {
                        return Err(skill_error("focused agent Project changed; refresh Skills"));
                    }
                }
                let snapshot = service.storage.snapshot()?;
                let native = match agent_id.as_deref() {
                    Some(id) => self
                        .agent_native_entries(id, snapshot.revision, false)?
                        .map(|native| native.entries)
                        .unwrap_or_default(),
                    None => vec![],
                };
                let revision = snapshot.revision;
                let skills =
                    crate::skill::resolver::projections(&snapshot, project_id.as_deref(), &native);
                let snapshot = service.storage.snapshot()?;
                if snapshot.revision != revision {
                    return Err(skill_error("catalog changed; refresh Skills"));
                }
                let packages=snapshot.catalogs.into_iter().filter(|catalog|catalog.scope==SkillScope::Chariox || matches!(&catalog.scope,SkillScope::Project {project_id:id} if Some(id)==project_id.as_ref())).flat_map(|catalog|catalog.packages).collect();
                Ok(SkillsV2Response::Catalog {
                    revision,
                    skills,
                    packages,
                    native,
                    metadata_budget_percent: self
                        .owned
                        .config_projection
                        .snapshot()
                        .user_config
                        .skills
                        .metadata_budget_percent
                        .get(),
                })
            }
            SkillsV2Request::Get {
                skill_id,
                project_id,
            } => {
                self.ensure_skill_project_visible(project_id.as_deref(), user)?;
                let (_, rows) = service.projections(project_id.as_deref(), &[])?;
                let skill = rows
                    .into_iter()
                    .find(|row| row.skill.id == skill_id)
                    .ok_or_else(|| {
                        skill_error("skill was removed or is outside this Project; refresh Skills")
                    })?;
                let (body, _) = verified_body(&service, &skill.skill)?;
                Ok(SkillsV2Response::Detail { skill, body })
            }
            SkillsV2Request::Preview { mutation } => {
                self.ensure_skill_mutation_owner(&mutation, user)?;
                let native = self.native_review_sources(&mutation, user, origin)?;
                let workspace = if let Some(origin) = origin {
                    let agent = self.owned.agent_store.get_agent(&origin.agent_id)?;
                    let session = self.owned.session_store.get_session(&origin.session_id)?;
                    if agent.remote_execution().is_some() {
                        None
                    } else {
                        Some(PathBuf::from(
                            agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                        ))
                    }
                } else {
                    None
                };
                let access = if origin.is_some() {
                    crate::skill::source_access::SkillSourceAccess::Agent {
                        workspace: workspace.as_deref(),
                    }
                } else {
                    crate::skill::source_access::SkillSourceAccess::Owner
                };
                service.preview_review_with_source_access(user, &mutation, &native, access)
            }
            SkillsV2Request::ResolveRepository { url } => {
                if origin.is_some() || !self.skills_host_owner(user) {
                    return Err(skill_error(
                        "only the home kernel owner can import a repository, from Skills",
                    ));
                }
                service.resolve_repository(&url)
            }
            SkillsV2Request::ResolveRepositoryUpdate { package_id } => {
                if origin.is_some() || !self.skills_host_owner(user) {
                    return Err(skill_error(
                        "only the home kernel owner can update a repository package, from Skills",
                    ));
                }
                service.resolve_repository_update(&package_id)
            }
            SkillsV2Request::Mutate {
                mutation,
                review_receipt_id,
            } => {
                self.ensure_skill_mutation_owner(&mutation, user)?;
                let (_, affected) =
                    service.reviewed_changes(user, &mutation, &review_receipt_id)?;
                self.authorize_current_external_command()?;
                let published = self.with_skill_tool_origin(origin, || {
                    service.publish_reviewed(user, &mutation, &review_receipt_id)
                })?;
                let (removed_skill_ids, changed_skill_ids): (Vec<_>, Vec<_>) = affected
                    .into_iter()
                    .map(|record| record.id)
                    .partition(|id| published.record(id).is_none());
                self.owned.runtime_projection_changes.record_change();
                Ok(SkillsV2Response::Published {
                    revision: published.revision,
                    changed_skill_ids,
                    removed_skill_ids,
                })
            }
            SkillsV2Request::Loads {
                session_id,
                agent_id,
            } => {
                let session = self.owned.session_store.get_session(&session_id)?;
                if !self.skills_host_owner(user) && !session.has_member(user) {
                    return Err(skill_error(
                        "session membership is required to view skill deliveries",
                    ));
                }
                if let Some(id) = &agent_id {
                    self.skill_agent_session(&session_id, id, user)?;
                }
                let records = service
                    .storage
                    .loads(&session_id)?
                    .into_iter()
                    .filter(|record| agent_id.as_ref().is_none_or(|id| id == &record.agent_id))
                    .collect();
                Ok(SkillsV2Response::Loads { records })
            }
            SkillsV2Request::List {
                session_id,
                agent_id,
                cursor,
                limit,
            } => {
                let session = self.skill_agent_session(&session_id, &agent_id, user)?;
                let prompt = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &agent_id)
                    .filter(|prompt| origin.is_none_or(|origin| prompt.id() == origin.prompt_id))
                    .ok_or_else(|| {
                        skill_error("list_skills requires the current kernel-owned turn")
                    })?;
                let authority = prompt
                    .skill_turn_authority()
                    .ok_or_else(|| skill_error("this turn has no admitted skill catalog"))?;
                if authority.project_id != session.project_id() {
                    return Err(skill_error(
                        "Project changed during this turn; submit a new turn before loading skills",
                    ));
                }
                let catalog = service
                    .storage
                    .snapshot_at_revision(authority.catalog_revision)?;
                let revision = catalog.revision;
                let native = self.turn_native_entries(authority, &agent_id)?;
                let rows = crate::skill::resolver::projections(
                    &catalog,
                    Some(session.project_id()),
                    &native,
                );
                let (skills, next_cursor) =
                    discovery_page(&rows, revision, cursor.as_deref(), limit)?;
                Ok(SkillsV2Response::Page {
                    revision,
                    skills,
                    next_cursor,
                })
            }
            SkillsV2Request::Load {
                session_id,
                agent_id,
                skill_id,
            } => {
                let session = self.skill_agent_session(&session_id, &agent_id, user)?;
                let prompt = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &agent_id)
                    .filter(|prompt| origin.is_none_or(|origin| prompt.id() == origin.prompt_id))
                    .ok_or_else(|| {
                        skill_error("load_skill requires a current kernel-owned provider turn")
                    })?;
                let authority = prompt.skill_turn_authority().ok_or_else(|| {
                    skill_error("this turn has no admitted skill catalog; submit a new turn")
                })?;
                if authority.project_id != session.project_id() {
                    return Err(skill_error(
                        "Project changed during this turn; submit a new turn before loading skills",
                    ));
                }
                let catalog = service
                    .storage
                    .snapshot_at_revision(authority.catalog_revision)?;
                let native = self.turn_native_entries(authority, &agent_id)?;
                let rows = crate::skill::resolver::projections(
                    &catalog,
                    Some(session.project_id()),
                    &native,
                );
                let explicit = authority.explicit_skill_ids.clone();
                let current = service.storage.snapshot()?;
                let current_rows =
                    crate::skill::resolver::projections(&current, Some(session.project_id()), &[]);
                admit_load(&current_rows, &skill_id, &explicit)?;
                let row = admit_load(&rows, &skill_id, &explicit)?;
                if let SkillDelivery::ProvidedBy { provider, .. } = &row.delivery {
                    return Err(skill_error(&format!("this instruction is provided by {provider}; use the supported native invocation instead of loading a duplicate Chariox body")));
                }
                if authority.prompt_delivery_skill_ids.contains(&skill_id)
                    || service.storage.loads(&session_id)?.iter().any(|loaded| {
                        loaded.agent_id == agent_id
                            && loaded.run_id == authority.provider_run_id
                            && loaded.context_generation == authority.context_generation
                            && loaded.skill_id == skill_id
                            && loaded.digest == row.skill.digest
                    })
                {
                    return Err(skill_error("these instructions were already supplied in the current context; continue using that body instead of loading a duplicate"));
                }
                let (body, mut root) = verified_body(&service, &row.skill)?;
                if let Some(remote_root) = authority.remote_roots.get(&skill_id) {
                    // The leased agent reads the package where the worker verified it.
                    root = remote_root.clone();
                }
                let result = SkillsV2Response::Loaded {
                    skill_id: skill_id.clone(),
                    digest: row.skill.digest.clone(),
                    body,
                    root,
                    files: row.skill.files.clone(),
                };
                // The instruction tool uses one JSON text result. Count its
                // actual escaped body, paths and manifest, plus a wrapper bound.
                let response_bytes = serde_json::to_vec(&result)
                    .map_err(|_| skill_error("cannot encode instruction body"))?
                    .len() as u64
                    + 256;
                let remaining_context = self.turn_remaining_context(authority);
                let record = SkillLoadRecord {
                    skill_id: skill_id.clone(),
                    digest: row.skill.digest.clone(),
                    session_id: session_id.clone(),
                    agent_id: agent_id.clone(),
                    run_id: authority.provider_run_id.clone(),
                    prompt_id: prompt.id().into(),
                    reason: if explicit.contains(&skill_id) {
                        SkillLoadReason::Explicit
                    } else {
                        SkillLoadReason::Model
                    },
                    context_generation: authority.context_generation.clone(),
                    delivered_at_ms: 0,
                };
                self.owned.reserve_skill_body(
                    authority,
                    response_bytes,
                    &record,
                    remaining_context,
                )?;
                // Returning a body is not a Loaded receipt. The provider/runtime
                // response acknowledgement owns recording actual delivery.
                Ok(result)
            }
            SkillsV2Request::NativeInventory { agent_id, refresh } => {
                let agent = self.owned.agent_store.get_agent(&agent_id)?;
                self.skill_agent_session(agent.session_id(), &agent_id, user)?;
                let inventory = self
                    .agent_native_entries(&agent_id, service.storage.snapshot()?.revision, refresh)?
                    .ok_or_else(|| {
                        skill_error("native inventory requires the focused provider to start; submit a turn or attach its native TUI")
                    })?;
                Ok(SkillsV2Response::NativeInventory {
                    entries: inventory.entries,
                    inventory_revision: inventory.revision,
                })
            }
            SkillsV2Request::NativeDetails {
                agent_id,
                selection,
            } => {
                let agent = self.owned.agent_store.get_agent(&agent_id)?;
                self.skill_agent_session(agent.session_id(), &agent_id, user)?;
                if agent.remote_execution().is_some() {
                    let revision = service.storage.snapshot()?.revision;
                    let (_, details) = tokio::runtime::Handle::current().block_on(
                        self.recovering_remote_skill_native_inventory(
                            &agent,
                            revision,
                            false,
                            Some(selection),
                        ),
                    )?;
                    let details = details.ok_or_else(|| {
                        skill_error("native inventory requires the focused provider to start; submit a turn or attach its native TUI")
                    })?;
                    return Ok(SkillsV2Response::NativeDetails { details });
                }
                let inventory = self.local_skill_inventory(
                    &agent_id,
                    service.storage.snapshot()?.revision,
                    false,
                )?;
                let details = super::skills_native_inventory::native_details(
                    &inventory,
                    &selection,
                    self.owned
                        .config_projection
                        .snapshot()
                        .host_machine_id
                        .clone(),
                )?;
                Ok(SkillsV2Response::NativeDetails { details })
            }
            SkillsV2Request::Submit { .. } => Err(skill_error(
                "source-aware submission must use the ordinary agent prompt lane",
            )),
        }
    }
}
fn verified_body(
    service: &SkillCatalogService,
    record: &SkillRecord,
) -> Result<(String, String), DaemonError> {
    let root = service.verified_package_root(record)?;
    let package = crate::skill::package_skill_directory(&root)?;
    if package.version_hash != record.digest {
        return Err(skill_error(
            "immutable skill package changed; reinstall before loading",
        ));
    }
    let file = package
        .files
        .iter()
        .find(|file| file.path == "SKILL.md")
        .ok_or_else(|| skill_error("verified skill has no SKILL.md"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.content_base64)
        .map_err(|_| skill_error("invalid verified skill encoding"))?;
    let body =
        String::from_utf8(bytes).map_err(|_| skill_error("skill instructions must be UTF-8"))?;
    Ok((body, root.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn skill_runtime_capability_follows_readiness_without_changing_other_controls() {
        let closed = capabilities_for_readiness(false);
        assert_eq!(
            closed,
            crate::local::RUNTIME_CONTROL_CAPABILITIES
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        );
        let ready = capabilities_for_readiness(true);
        assert_eq!(
            ready
                .iter()
                .filter(|value| value.as_str() == crate::local::SKILLS_V2_RUNTIME_CAPABILITY)
                .count(),
            1
        );
        assert_eq!(
            ready
                .iter()
                .filter(|value| value.as_str() == crate::local::SKILLS_REPOSITORY_UPDATE_CAPABILITY)
                .count(),
            1
        );
        assert!(closed.iter().all(|value| ready.contains(value)));
        assert_eq!(ready.len(), closed.len() + 2);
    }
}

#[cfg(test)]
mod owner_gate_tests {
    use super::*;

    #[tokio::test]
    async fn catalog_owner_requires_the_authenticated_relay_principal() {
        use crate::runtime::command::{
            KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource,
        };
        let mut config = crate::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner".into(),
            realm_id: "owner-realm".into(),
            ..Default::default()
        });
        let app = crate::DaemonApp::bootstrap(config).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        crate::test_support::install_skill_through_requests(&router, "owner-skill", 0).await;
        for (kind, user, realm, allowed) in [
            (
                KernelCallerKind::RemoteClient,
                "owner",
                Some("wrong-realm"),
                false,
            ),
            (KernelCallerKind::RemoteClient, "owner", None, false),
            (
                KernelCallerKind::RemoteKernel,
                "owner",
                Some("owner-realm"),
                false,
            ),
            (
                KernelCallerKind::HostedService,
                "owner",
                Some("owner-realm"),
                false,
            ),
            (
                KernelCallerKind::LocalClient,
                "local",
                Some("owner-realm"),
                false,
            ),
            (
                KernelCallerKind::RemoteClient,
                "local",
                Some("owner-realm"),
                false,
            ),
            (
                KernelCallerKind::RemoteClient,
                "member",
                Some("owner-realm"),
                false,
            ),
            (
                KernelCallerKind::RemoteClient,
                "owner",
                Some("owner-realm"),
                true,
            ),
        ] {
            let hosted_service = kind == KernelCallerKind::HostedService;
            let caller = KernelCaller {
                caller_kind: kind,
                user_id: Some(user.into()),
                realm_id: realm.map(str::to_owned),
                ..KernelCaller::for_source(&KernelCommandSource::RelayClient)
            };
            for request in [
                SkillsV2Request::Preview {
                    mutation: install_probe("probe"),
                },
                SkillsV2Request::Mutate {
                    mutation: install_probe("probe"),
                    review_receipt_id: "not-issued".into(),
                },
            ] {
                let preview = matches!(request, SkillsV2Request::Preview { .. });
                let request = LocalDaemonRequest::SkillsV2(request);
                let command = KernelCommand::from_local_request_with_caller(
                    "owner-probe",
                    KernelCommandSource::RelayClient,
                    caller.clone(),
                    None,
                    None,
                    &request,
                );
                let result = router.dispatch(command, request).await;
                if allowed {
                    if preview {
                        assert!(matches!(
                            result.unwrap(),
                            crate::local::LocalDaemonResponse::SkillsV2 {
                                result: SkillsV2Response::Preview { .. }
                            }
                        ));
                    } else if let Err(error) = result {
                        assert!(!error.to_string().contains("home kernel owner"), "{error}");
                    }
                } else {
                    let error =
                        result.expect_err("a user ID alone must not grant catalog ownership");
                    assert!(
                        error.to_string().contains("home kernel owner")
                            || (hosted_service
                                && error
                                    .to_string()
                                    .contains("authorize hosted service request")),
                        "{error}"
                    );
                }
            }
        }
        let snapshot = router
            .runtime_state()
            .skills_catalog_service()
            .unwrap()
            .storage
            .snapshot()
            .unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot
                .catalogs
                .iter()
                .map(|catalog| catalog.skills.len())
                .sum::<usize>(),
            1
        );
    }

    fn install_probe(name: &str) -> SkillCatalogMutation {
        SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 1,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content {
                    skill_md: format!("---\nname: {name}\ndescription: Probe {name}\n---\nBody.\n"),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        }
    }

    // A shared-session member is not the home kernel owner: Preview and Mutate are
    // refused before any review or publish, and the catalog keeps the owner's revision.
    #[tokio::test]
    async fn session_member_cannot_preview_or_mutate_the_home_catalog() {
        let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let revision =
            crate::test_support::install_skill_through_requests(&router, "owner-skill", 0).await;
        assert_eq!(revision, 1);
        let member = "shared-session-member".to_owned();
        let denial = "the home kernel owner must authorize skill catalog mutations";
        let preview = state
            .execute_skills_catalog_request(
                SkillsV2Request::Preview {
                    mutation: install_probe("member-skill"),
                },
                member.clone(),
            )
            .await
            .unwrap_err();
        assert!(preview.to_string().contains(denial), "{preview}");
        let mutate = state
            .execute_skills_catalog_request(
                SkillsV2Request::Mutate {
                    mutation: install_probe("member-skill"),
                    review_receipt_id: "not-issued".into(),
                },
                member,
            )
            .await
            .unwrap_err();
        assert!(mutate.to_string().contains(denial), "{mutate}");
        let crate::local::LocalDaemonResponse::SkillsV2 {
            result: SkillsV2Response::Catalog {
                revision, skills, ..
            },
        } = state
            .execute_skills_catalog_request(
                SkillsV2Request::Catalog {
                    project_id: None,
                    agent_id: None,
                },
                crate::session::DEFAULT_LOCAL_USER_ID.to_owned(),
            )
            .await
            .unwrap()
        else {
            panic!("Catalog must answer with the catalog");
        };
        assert_eq!(revision, 1);
        assert_eq!(skills.len(), 1);
    }
}
