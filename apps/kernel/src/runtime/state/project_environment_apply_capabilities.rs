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
                let installations =
                    environment_app_installations(&self.owned.durable_state_store, owner)?;
                installations.iter().any(|a| {
                    a.app_id == *app_id
                        && a.active
                            .as_ref()
                            .is_some_and(|a| a.release.package_digest == *digest)
                        && agent.has_extension_grant(
                            crate::extension::ExtensionKind::App,
                            &a.installation_id,
                        )
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
                let installations =
                    environment_app_installations(&self.owned.durable_state_store, owner)?;
                let agent = self.owned.agent_store.get_agent(agent_id)?;
                let installation = installations
                    .iter()
                    .filter(|a| {
                        a.app_id == *app_id
                            && a.active
                                .as_ref()
                                .is_some_and(|a| a.release.package_digest == *digest)
                    })
                    .min_by_key(|installation| {
                        !agent.has_extension_grant(
                            crate::extension::ExtensionKind::App,
                            &installation.installation_id,
                        )
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r4_environment_apply_keeps_live_room_app_grant_and_admits_next_prompt() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("r4-room-app-apply");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let _mcp = crate::test_support::TestRuntimeMcp::serve(&mut config);
        let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        for index in 0..100 {
            crate::durable_state::app_state::fixture_installation(
                &app.durable_state_store(),
                "alice",
                &format!("000-fill-{index:03}"),
            );
        }
        let fixture_store = app.durable_state_store();
        let (bytes, publisher) = crate::durable_state::app_state::fixture_event_package();
        let package = chariox_app_package::verify(
            &bytes,
            &chariox_app_package::VerificationPolicy::new(
                crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                vec![publisher],
            ),
        )
        .unwrap();
        chariox_app_runtime::release_store::ReleaseStore::open_or_create(fixture_store.path())
            .unwrap()
            .stage(
                &package,
                &bytes,
                chariox_app_runtime::release_store::StageBudget {
                    max_stage_bytes: 1024 * 1024,
                    reserved_bytes: 1024 * 1024,
                    host_reserve_bytes: 1024 * 1024,
                },
            )
            .unwrap();
        fixture_store
            .active_app_capabilities("alice", "installed")
            .unwrap();
        let profile = app
            .provider_account_profile_registry()
            .create_managed("alice", "opencode", "Room App prompt fixture")
            .unwrap();
        let environment = app
            .provider_account_profile_registry()
            .resolve_environment("alice", "opencode", &profile.profile_id)
            .unwrap();
        let auth_directory = std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode");
        std::fs::create_dir_all(&auth_directory).unwrap();
        crate::config::write_private_file(
            &auth_directory.join("auth.json"),
            br#"{"openai":{"type":"api","key":"fixture-only-not-a-real-key"}}"#,
        )
        .unwrap();
        app.provider_account_profile_registry()
            .update_observation(
                "alice",
                "opencode",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                root.session_request()
                    .with_owner_user_id("alice")
                    .with_agent_defaults(
                        crate::session::SessionAgentDefaults::new("opencode")
                            .with_account_profile(profile.profile_id),
                    ),
            )
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "r4-owner",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "alice",
            ))
            .unwrap();
        let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        )
        .runtime_state();
        let bound = state
            .grant_agent_extension(
                agent.id(),
                crate::extension::ExtensionGrant::app("installed"),
                "alice",
            )
            .await
            .unwrap();
        assert!(bound
            .extension_grants()
            .iter()
            .any(|grant| grant.name == "installed" && grant.app_grant.is_some()));
        let installation = state
            .owned
            .durable_state_store
            .get_app_installation("alice", "installed")
            .unwrap();
        let project = state
            .owned
            .session_store
            .get_project(session.project_id())
            .unwrap();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let current = store.snapshot(&project).unwrap();
        let mut draft = environment_draft(&current);
        draft.project_requirements.push(Requirement {
            requirement_id: "app".into(),
            title: "Admitted App".into(),
            scope: RequirementScope::Project,
            origins: vec![],
            spec: RequirementSpec::CharioxApps {
                app_id: installation.app_id,
                version: None,
                content_digest: Some(installation.active.unwrap().release.package_digest),
                required_grants: vec![],
            },
            depends_on: vec![],
            platform_variants: vec![],
            required: true,
            legacy_entry: None,
        });
        let env = store
            .save_revision_locked(
                &current,
                &crate::local::SaveProjectEnvironmentRevisionRequest {
                    project_id: project.id().into(),
                    expected_revision: current.revision,
                    expected_content_digest: current.content_digest.clone(),
                    draft,
                    accepted_proposal_ids: vec![],
                    excluded_proposal_ids: vec![],
                },
                "alice",
            )
            .unwrap()
            .0;
        let mut wrong_package = env.project_requirements[0].clone();
        if let RequirementSpec::CharioxApps { content_digest, .. } = &mut wrong_package.spec {
            *content_digest = Some("f".repeat(64))
        }
        assert!(!state
            .environment_capability_already_granted(&wrong_package, agent.id(), "alice")
            .unwrap());
        let target = current_check_target(&config);
        let LocalDaemonResponse::ProjectEnvironmentPlan { plan } = state
            .plan_project_environment(
                crate::local::PlanProjectEnvironmentRequest {
                    project_id: project.id().into(),
                    expected_revision: env.revision,
                    revision_digest: env.content_digest.clone(),
                    target: target.clone(),
                    selected_items: vec![],
                },
                "alice",
            )
            .await
            .unwrap()
        else {
            panic!("Plan expected")
        };
        state
            .apply_project_environment(
                crate::local::ApplyProjectEnvironmentRequest {
                    project_id: project.id().into(),
                    expected_revision: env.revision,
                    revision_digest: env.content_digest.clone(),
                    target,
                    operation_id: "room-app".into(),
                    plan_id: plan.plan_id,
                    selected_items: plan.selected_items,
                    per_item_opt_ins: plan
                        .required_opt_ins
                        .into_iter()
                        .map(|id| EnvironmentItemOptIn {
                            requirement_id: id,
                            decision: EnvironmentOptInDecision::Accepted,
                            user_id: "alice".into(),
                            recorded_at_ms: crate::session::unix_epoch_ms(),
                        })
                        .collect(),
                },
                "alice",
            )
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut settled = false;
        while std::time::Instant::now() < deadline {
            let journal = store.load_applies(project.id()).unwrap();
            if !apply_active(&journal.attempts.last().unwrap().operation) {
                settled = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if !settled {
            state
                .cancel_environment_operation(
                    crate::local::EnvironmentOperationRequest {
                        project_id: project.id().into(),
                        operation_id: "room-app".into(),
                    },
                    "alice",
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        let operation = store
            .load_applies(project.id())
            .unwrap()
            .attempts
            .last()
            .unwrap()
            .operation
            .clone();
        let prompt = if operation.phase == EnvironmentOperationPhase::Ready {
            Some(
                state
                    .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                        session_id: session.id().into(),
                        prompt: crate::session::PromptQueueItem::new(
                            "r4-next-prompt",
                            attachment.id(),
                            agent.id(),
                            "Continue with the applied App",
                            crate::session::PromptStatus::Queued,
                        ),
                        force_queue: true,
                        refresh_projection: true,
                    })
                    .await,
            )
        } else {
            None
        };
        let app_grants: Vec<_> = state
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .extension_grants()
            .iter()
            .filter(|grant| grant.kind == crate::extension::ExtensionKind::App)
            .map(|grant| grant.name.clone())
            .collect();
        state.delete_session_id(session.id()).await.unwrap();
        assert!(
            settled,
            "Apply asked for an existing live Room App grant again"
        );
        assert_eq!(
            operation.phase,
            EnvironmentOperationPhase::Ready,
            "{:?}",
            operation.per_item_results
        );
        assert_eq!(operation.receipts.len(), 1);
        assert_eq!(
            app_grants,
            vec!["installed".to_owned()],
            "Apply must retain the existing installation binding"
        );
        let prompt_error = prompt.unwrap().err().map(|error| error.to_string());
        assert!(
            prompt_error.is_none(),
            "the next idle prompt must pass applied App binding validation: {prompt_error:?}"
        );
    }
}
