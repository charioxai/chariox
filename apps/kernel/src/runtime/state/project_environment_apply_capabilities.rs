//! MP-08 / MP-10 / MP-11: scoped references enter agents only through existing admission/grant services.
use super::*;
use crate::project_environment::*;

fn matching_scope(
    env: &ProjectEnvironment,
    r: &Requirement,
    session: &crate::session::RuntimeSession,
) -> bool {
    match &r.scope {
        RequirementScope::Project => true,
        RequirementScope::Folder { folder_id } => env
            .folders
            .iter()
            .find(|f| &f.folder_id == folder_id)
            .is_some_and(|f| f.local_workspace_binding == session.workspace_id()),
    }
}
impl KernelRuntimeState {
    // MP-11: names-only guidance uses the existing human-only interaction
    // service. Answers never carry a value or authorize a new Vault/grant path.
    // Unrelated eligible items have already settled before this notification.
    pub(super) fn notify_environment_named_inputs(
        &self,
        env: ProjectEnvironment,
        operation: EnvironmentOperation,
        owner: String,
    ) {
        let names: Vec<_> = environment_requirements(&env)
            .filter(|r| {
                operation.per_item_results.iter().any(|result| {
                    result.requirement_id == r.requirement_id
                        && matches!(
                            result.status,
                            EnvironmentObservationStatus::Missing
                                | EnvironmentObservationStatus::NeedsYourInput
                        )
                })
            })
            .filter_map(|r| match &r.spec {
                RequirementSpec::Variables { name, .. } | RequirementSpec::Secrets { name, .. } => {
                    Some(name.clone())
                }
                _ => None,
            })
            .filter(|name| {
                name.len() <= 128 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .collect();
        if names.is_empty() {
            return;
        }
        let Some(session) = self
            .owned
            .session_store
            .sessions_in_project(&env.local_project_id)
            .into_iter()
            .find(|s| s.owner_user_id() == owner)
        else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            if state.authorize_current_external_command().is_err() {
                return;
            }
            let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
                format!("apply-inputs-{:032x}", rand::random::<u128>()),
                format!("environment-inputs:{}", operation.operation_id),
                "Review Project inputs",
                format!("Missing Project input names: {}. Open the existing Vault or selected public sources, then Check and explicitly retry. This decision never asks for or stores a secret value. Unrelated scoped work has already been recorded.", names.join(", ")),
                vec![crate::session::RuntimeInteractionChoice::new("review", "Review inputs", "review", None)],
            ).with_timeout_sec(60);
            if let Ok(answer) = state
                .create_kernel_operation_interaction(session.id(), &owner, interaction)
                .await
            {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(60), answer).await;
            }
        });
    }
    pub(super) async fn apply_project_capability(
        &self,
        env: &ProjectEnvironment,
        r: &Requirement,
        owner: &str,
        operation: &EnvironmentOperation,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), DaemonError> {
        for session in self
            .owned
            .session_store
            .sessions_in_project(&env.local_project_id)
        {
            if !matching_scope(env, r, &session) {
                continue;
            }
            for agent in session.agents() {
                if agent.owner_user_id() == owner {
                    if !self.environment_capability_already_granted(r, agent.id(), owner)? {
                        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
                            format!("apply-grant-{:032x}", rand::random::<u128>()),
                            format!("environment-apply:{}", operation.operation_id),
                            "Allow Project Agent tools",
                            format!("Apply needs a new capability grant for requirement {} and agent {}. Approve this named grant through the existing capability service; no secret value is requested here.", r.requirement_id, agent.id()),
                            vec![crate::session::RuntimeInteractionChoice::new("approve", "Allow", "approve", None), crate::session::RuntimeInteractionChoice::new("decline", "Decline", "decline", None)],
                        ).with_timeout_sec(60);
                        let mut answer = self
                            .create_kernel_operation_interaction(session.id(), owner, interaction)
                            .await?;
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(60);
                        loop {
                            if cancelled() || std::time::Instant::now() >= deadline {
                                return Err(environment_error(
                                    "Capability authorization cancelled or timed out",
                                ));
                            }
                            tokio::select! {
                                response = &mut answer => { if !response.is_ok_and(|reply| reply.status == "answered" && reply.choice_id.as_deref() == Some("approve")) { return Err(environment_error("Capability grant declined")); } break; }
                                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                            }
                        }
                    }
                    if cancelled() {
                        return Err(environment_error("Capability Apply cancelled"));
                    }
                    self.bind_environment_capability(env, r, agent.id(), owner)
                        .await?;
                }
            }
        }
        Ok(())
    }
    fn environment_capability_already_granted(
        &self,
        r: &Requirement,
        agent_id: &str,
        owner: &str,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        Ok(match &r.spec {
            RequirementSpec::AgentTools {
                tool_kind: AgentToolKind::Mcp,
                registry_ref,
                ..
            } => agent.mcp_grants().contains(registry_ref),
            RequirementSpec::AgentTools { registry_ref, .. } => {
                agent.skill_grants().contains(registry_ref)
            }
            RequirementSpec::CharioxApps {
                app_id,
                content_digest: Some(digest),
                ..
            } => {
                let installations = self
                    .owned
                    .durable_state_store
                    .list_app_installations(owner, None, 1000)
                    .map_err(|_| environment_error("App registry unavailable"))?;
                installations.installations.iter().any(|a| {
                    a.app_id == *app_id
                        && a.active
                            .as_ref()
                            .is_some_and(|a| a.release.package_digest == *digest)
                        && agent
                            .extension_grants()
                            .contains(&crate::extension::ExtensionGrant::app(&a.installation_id))
                })
            }
            _ => false,
        })
    }
    async fn bind_environment_capability(
        &self,
        env: &ProjectEnvironment,
        r: &Requirement,
        agent_id: &str,
        owner: &str,
    ) -> Result<(), DaemonError> {
        self.authorize_current_external_command()?;
        let project = self
            .owned
            .session_store
            .get_project(&env.local_project_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(agent.session_id())?;
        if project.owner_user_id() != owner
            || agent.owner_user_id() != owner
            || session.project_id() != project.id()
            || !matching_scope(env, r, &session)
        {
            return Err(environment_error("Project capability scope changed"));
        }
        let config = self.owned.config_projection.snapshot();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        if store.snapshot(&project)?.content_digest != env.content_digest {
            return Err(environment_error("Project capability revision changed"));
        }
        let ctx = EnvironmentCheckContext {
            config: config.clone(),
            owner: owner.into(),
            profiles: self.owned.provider_account_profiles.clone(),
            apps: self.owned.durable_state_store.clone(),
        };
        let checked = check_requirement(
            &ctx,
            env,
            r,
            &current_check_target(&config),
            "apply-capability",
            &mut BTreeMap::new(),
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        );
        if checked.status != EnvironmentObservationStatus::Ready {
            return Err(environment_error(
                "Capability admission changed; Check and review it",
            ));
        }
        match &r.spec {
            RequirementSpec::AgentTools {
                tool_kind,
                registry_ref,
                package_digest: Some(_),
                ..
            } => match tool_kind {
                AgentToolKind::Mcp => {
                    self.grant_agent_mcp(agent_id, registry_ref.clone(), owner)
                        .await?;
                }
                AgentToolKind::Skill | AgentToolKind::InstructionFile => {
                    self.grant_agent_skill(agent_id, registry_ref.clone(), owner)
                        .await?;
                }
            },
            RequirementSpec::CharioxApps {
                app_id,
                content_digest: Some(digest),
                ..
            } => {
                let installations = self
                    .owned
                    .durable_state_store
                    .list_app_installations(owner, None, 1000)
                    .map_err(|_| environment_error("App registry unavailable"))?;
                let installation = installations
                    .installations
                    .iter()
                    .find(|a| {
                        a.app_id == *app_id
                            && a.active
                                .as_ref()
                                .is_some_and(|a| a.release.package_digest == *digest)
                    })
                    .ok_or_else(|| environment_error("Admitted App generation unavailable"))?;
                self.grant_agent_extension(
                    agent_id,
                    crate::extension::ExtensionGrant::app(&installation.installation_id),
                    owner,
                )
                .await?;
            }
            _ => {
                return Err(environment_error(
                    "Pin the reviewed capability package before Apply",
                ))
            }
        }
        Ok(())
    }
    pub(super) fn apply_environment_agent_bindings(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let project = self.owned.session_store.get_project(session.project_id())?;
        let config = self.owned.config_projection.snapshot();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let env = store.snapshot(&project)?;
        let applied = store.load_applies(project.id())?;
        for r in environment_requirements(&env).filter(|r| matching_scope(&env, r, &session)) {
            if !matches!(
                &r.spec,
                RequirementSpec::AgentTools { .. } | RequirementSpec::CharioxApps { .. }
            ) {
                continue;
            }
            let authorized = applied.attempts.iter().any(|a| {
                a.operation.revision_digest == env.content_digest
                    && a.operation.target.machine_id == config.host_machine_id
                    && a.operation.target.slice_ref.is_none()
                    && a.operation.receipts.iter().any(|receipt| {
                        receipt.requirement_id.as_deref() == Some(&r.requirement_id)
                            && receipt.artifact_digest == metadata_digest(&r.spec)
                    })
            });
            if authorized
                && !self.environment_capability_already_granted(
                    r,
                    agent_id,
                    project.owner_user_id(),
                )?
            {
                return Err(environment_error(
                    "Project capability requires normal Agent-tool authorization for this agent",
                ));
            }
        }
        Ok(())
    }
}
