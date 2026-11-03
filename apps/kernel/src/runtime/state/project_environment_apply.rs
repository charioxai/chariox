//! MP-08 / MP-10 / MP-11: Project-owned Apply admission and recovery, shared by real clients.
use super::*;
use crate::project_environment::*;

#[path = "project_environment_apply_runner.rs"]
mod runner;

impl KernelRuntimeState {
    fn apply_context(
        &self,
        project_id: &str,
        user: &str,
    ) -> Result<
        (
            crate::session::RuntimeProject,
            ProjectEnvironmentStore,
            EnvironmentCheckContext,
        ),
        DaemonError,
    > {
        let project = self.owned.session_store.get_project(project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "Caller does not own the selected Project",
            ));
        }
        let config = self.owned.config_projection.snapshot();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let ctx = EnvironmentCheckContext {
            config,
            owner: user.into(),
            profiles: self.owned.provider_account_profiles.clone(),
            apps: self.owned.durable_state_store.clone(),
        };
        Ok((project, store, ctx))
    }
    pub(crate) async fn plan_project_environment(
        &self,
        request: crate::local::PlanProjectEnvironmentRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let (project, store, ctx) = self.apply_context(&request.project_id, user)?;
        let target = apply_target(&ctx.config, &request.target)?;
        let _lock = store.lock_briefly_async(project.id()).await?;
        let (project, _, _) = self.apply_context(&request.project_id, user)?;
        let env = store.snapshot_locked(&project)?;
        validate_revision(&env, request.expected_revision, &request.revision_digest)?;
        drop(_lock);
        let state = self.clone();
        let owner = user.to_owned();
        let plan = tokio::task::spawn_blocking(move || {
            let selected = apply_selection(&env, &request.selected_items)?;
            let binding_scopes = if request.selected_items.is_empty() {
                std::iter::once(RequirementScope::Project)
                    .chain(env.folders.iter().map(|f| RequirementScope::Folder {
                        folder_id: f.folder_id.clone(),
                    }))
                    .collect()
            } else {
                environment_requirements(&env)
                    .filter(|r| selected.contains(&r.requirement_id))
                    .map(|r| r.scope.clone())
                    .collect()
            };
            let mut checks = store.load_checks(project.id())?;
            let mut observations = vec![];
            let mut workspace = CheckWorkspace::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            for r in environment_requirements(&env).filter(|r| selected.contains(&r.requirement_id))
            {
                if std::time::Instant::now() > deadline {
                    return Err(environment_error(
                        "Plan probe budget exceeded; select fewer items",
                    ));
                }
                validate_project(&state, &project, &owner, &env, &store)?;
                observations.push(check_requirement_in_workspace(
                    &ctx,
                    &env,
                    r,
                    &target,
                    "apply-plan",
                    &mut checks.public_bindings,
                    deadline,
                    &mut workspace,
                ));
            }
            let _lock = store.lock_briefly(project.id())?;
            validate_project(&state, &project, &owner, &env, &store)?;
            if current_check_target(&state.owned.config_projection.snapshot()) != target {
                return Err(environment_error(
                    "Plan target changed during probes; refresh and retry",
                ));
            }
            // Apply and Check can publish while Plan probes. Merge into their latest journal.
            let mut latest = store.load_checks(project.id())?;
            for mut observation in observations {
                let key = format!(
                    "{}:{}:{}",
                    observation.revision_digest, target.machine_id, observation.requirement_id
                );
                if checks
                    .public_bindings
                    .get(&key)
                    .zip(latest.public_bindings.get(&key))
                    .is_some_and(|(a, b)| a != b)
                {
                    observation.status = EnvironmentObservationStatus::Outdated;
                    observation.reason_code = "binding_drift".into();
                    observation.safe_summary =
                        "Public binding changed since its first Check for this revision".into();
                }
                let newer = latest.observations.iter().any(|current| {
                    current.requirement_id == observation.requirement_id
                        && current.target.machine_id == observation.target.machine_id
                        && current.target.slice_ref == observation.target.slice_ref
                        && current.checked_at_ms > observation.checked_at_ms
                });
                if !newer {
                    latest.record_observation(observation);
                }
            }
            for (key, digest) in checks.public_bindings {
                latest.public_bindings.entry(key).or_insert(digest);
            }
            let plan =
                build_apply_plan(&env, target, &request.selected_items, &latest.observations)?;
            latest.folder_binding_digest = folder_bindings_digest(&env);
            store.save_checks(project.id(), &latest)?;
            let mut journal = store.load_applies(project.id())?;
            journal
                .plans
                .retain(|p| p.plan.expires_at_ms > crate::session::unix_epoch_ms());
            if journal.plans.len() >= 16 {
                journal.plans.remove(0);
            }
            journal.plans.push(StoredApplyPlan {
                binding_scopes,
                plan: plan.clone(),
                folder_binding_digest: folder_bindings_digest(&env),
            });
            store.save_applies(project.id(), &journal)?;
            Ok::<_, DaemonError>(plan)
        })
        .await
        .map_err(|_| environment_error("Plan task failed"))??;
        Ok(LocalDaemonResponse::ProjectEnvironmentPlan { plan })
    }
    pub(crate) async fn apply_project_environment(
        &self,
        request: crate::local::ApplyProjectEnvironmentRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        validate_apply_id(&request.operation_id)?;
        let (project, store, ctx) = self.apply_context(&request.project_id, user)?;
        let current_target = current_check_target(&ctx.config);
        let lock = store.lock_briefly_async(project.id()).await?;
        let (project, _, _) = self.apply_context(&request.project_id, user)?;
        let env = store.snapshot_locked(&project)?;
        let mut journal = store.recover_applies(project.id(), &current_target)?;
        // Lost ACK replay remains idempotent even after plan expiry or a later edit.
        if let Some(operation) = replayed_apply_operation(&journal, &request)? {
            return Ok(LocalDaemonResponse::EnvironmentOperation { operation });
        }
        let request_digest = metadata_digest(&request);
        let target = apply_target(&ctx.config, &request.target)?;
        validate_revision(&env, request.expected_revision, &request.revision_digest)?;
        let plan = journal
            .plans
            .iter()
            .find(|p| p.plan.plan_id == request.plan_id)
            .ok_or_else(|| environment_error("Apply plan is unavailable; plan again"))?;
        if plan.plan.revision_digest != env.content_digest
            || plan.plan.target != target
            || plan.plan.selected_items != request.selected_items
            || plan.folder_binding_digest != folder_bindings_digest(&env)
            || plan.plan.expires_at_ms <= crate::session::unix_epoch_ms()
        {
            return Err(environment_error(
                "Apply plan changed or expired; plan again",
            ));
        }
        let binding_scopes = plan.binding_scopes.clone();
        let mut opted = std::collections::BTreeSet::new();
        if request.per_item_opt_ins.iter().any(|i| {
            !opted.insert(i.requirement_id.clone())
                || !plan.plan.required_opt_ins.contains(&i.requirement_id)
                || i.user_id != user
                || i.recorded_at_ms > crate::session::unix_epoch_ms()
        }) {
            return Err(environment_error(
                "Apply opt-ins must name this plan and its owner",
            ));
        }
        ensure_apply_idle(&journal)?;
        let now = crate::session::unix_epoch_ms();
        let operation = EnvironmentOperation {
            operation_id: request.operation_id,
            attempt: 1,
            local_project_id: project.id().into(),
            revision_digest: env.content_digest.clone(),
            target,
            kind: EnvironmentOperationKind::Apply,
            phase: EnvironmentOperationPhase::Requested,
            selected_items: request.selected_items,
            per_item_opt_ins: request.per_item_opt_ins,
            per_item_results: vec![],
            created_at_ms: now,
            updated_at_ms: now,
            cancellation: None,
            receipts: vec![],
            recovery_state: EnvironmentRecoveryState::Pending,
        };
        // The bounded durable journal must reject capacity before mutation,
        // never forget an accepted identity and admit a duplicate side effect.
        journal.attempts.push(ApplyAttempt {
            binding_scopes,
            plan_id: request.plan_id,
            request_digest: Some(request_digest),
            operation: operation.clone(),
            folder_binding_digest: folder_bindings_digest(&env),
            active_requirement: None,
            uncertain_requirements: vec![],
            setup_admitted_commands: Default::default(),
        });
        store.save_applies(project.id(), &journal)?;
        drop(lock);
        self.spawn_environment_apply(project, store, ctx, env, operation.clone());
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }
    pub(crate) async fn cancel_environment_operation(
        &self,
        request: crate::local::EnvironmentOperationRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let (project, store, ctx) = self.apply_context(&request.project_id, user)?;
        let _lock = store.lock_briefly_async(project.id()).await?;
        self.apply_context(&request.project_id, user)?;
        let mut journal =
            store.recover_applies(project.id(), &current_check_target(&ctx.config))?;
        let a = journal
            .attempts
            .iter_mut()
            .rev()
            .find(|a| a.operation.operation_id == request.operation_id)
            .ok_or_else(|| environment_error("Apply operation not found"))?;
        if apply_active(&a.operation) {
            a.operation
                .cancellation
                .get_or_insert(EnvironmentCancellation {
                    requested_at_ms: crate::session::unix_epoch_ms(),
                    settled_at_ms: None,
                });
        }
        let operation = a.operation.clone();
        store.save_applies(project.id(), &journal)?;
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }
    pub(crate) async fn retry_environment_operation(
        &self,
        request: crate::local::RetryEnvironmentOperationRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let (project, store, ctx) = self.apply_context(&request.project_id, user)?;
        let target = current_check_target(&ctx.config);
        let lock = store.lock_briefly_async(project.id()).await?;
        let (project, _, _) = self.apply_context(&request.project_id, user)?;
        let env = store.snapshot_locked(&project)?;
        let mut journal = store.recover_applies(project.id(), &target)?;
        let mut a = journal
            .attempts
            .iter()
            .rev()
            .find(|a| a.operation.operation_id == request.operation_id)
            .cloned()
            .ok_or_else(|| environment_error("Apply operation not found"))?;
        if a.operation.attempt == request.expected_attempt.saturating_add(1) {
            return Ok(LocalDaemonResponse::EnvironmentOperation {
                operation: a.operation,
            });
        }
        if a.operation.attempt != request.expected_attempt
            || !matches!(
                a.operation.phase,
                EnvironmentOperationPhase::Failed
                    | EnvironmentOperationPhase::Cancelled
                    | EnvironmentOperationPhase::AwaitingInput
            )
        {
            return Err(environment_error(
                "Apply attempt is active, settled or changed",
            ));
        }
        ensure_apply_idle(&journal)?;
        if a.operation.revision_digest != env.content_digest
            || a.folder_binding_digest != folder_bindings_digest(&env)
            || a.operation.target.machine_id != target.machine_id
            || a.operation.target.slice_ref != target.slice_ref
        {
            return Err(environment_error(
                "Apply revision or folder/target binding changed; make a new plan",
            ));
        }
        a.operation.attempt = a
            .operation
            .attempt
            .checked_add(1)
            .ok_or_else(|| environment_error("Apply attempt limit"))?;
        a.operation.target = target;
        a.operation.phase = EnvironmentOperationPhase::Requested;
        a.operation.cancellation = None;
        a.operation.per_item_results.clear();
        a.operation.updated_at_ms = crate::session::unix_epoch_ms();
        a.operation.recovery_state = EnvironmentRecoveryState::Pending;
        let operation = a.operation.clone();
        journal.attempts.push(a);
        store.save_applies(project.id(), &journal)?;
        drop(lock);
        self.spawn_environment_apply(project, store, ctx, env, operation.clone());
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }
}
fn apply_target(
    config: &crate::config::DaemonConfig,
    requested: &EnvironmentTargetBinding,
) -> Result<EnvironmentTargetBinding, DaemonError> {
    let target = current_check_target(config);
    let initial = EnvironmentTargetBinding {
        machine_id: config.host_machine_id.clone(),
        target_instance_generation: config.daemon_id.clone(),
        slice_ref: None,
    };
    if requested != &target && requested != &initial {
        return Err(environment_error(
            "Apply target generation is unavailable; select the owning kernel",
        ));
    }
    Ok(target)
}
fn validate_apply_id(id: &str) -> Result<(), DaemonError> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err(environment_error("Invalid Apply operation identity"));
    }
    Ok(())
}
fn validate_revision(
    env: &ProjectEnvironment,
    revision: u64,
    digest: &str,
) -> Result<(), DaemonError> {
    if env.revision == 0 || env.revision != revision || env.content_digest != digest {
        return Err(environment_error(
            "Save and review the current revision before Apply",
        ));
    }
    Ok(())
}
fn ensure_apply_idle(journal: &EnvironmentApplies) -> Result<(), DaemonError> {
    if journal.attempts.iter().any(|a| apply_active(&a.operation)) {
        return Err(environment_error("Apply already running for this Project"));
    }
    Ok(())
}
pub(in crate::runtime::state) fn validate_project(
    state: &KernelRuntimeState,
    project: &crate::session::RuntimeProject,
    owner: &str,
    env: &ProjectEnvironment,
    store: &ProjectEnvironmentStore,
) -> Result<(), DaemonError> {
    state.authorize_current_external_command()?;
    let current = state.owned.session_store.get_project(project.id())?;
    if current.owner_user_id() != owner || current.workspace_ids() != project.workspace_ids() || {
        let saved = store.snapshot_locked(&current)?;
        saved.revision != env.revision
            || saved.content_digest != env.content_digest
            || folder_bindings_digest(&saved) != folder_bindings_digest(env)
    } {
        return Err(environment_error(
            "Project ownership, revision or folder binding changed during Apply",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_environment_apply_concurrency_tests.rs"]
mod review0034;
