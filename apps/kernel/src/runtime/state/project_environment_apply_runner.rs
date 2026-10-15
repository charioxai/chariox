//! MP-08 / MP-10 / MP-11: durable item intent/receipt settlement through existing services.
use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_environment_apply(
        &self,
        project: crate::session::RuntimeProject,
        store: ProjectEnvironmentStore,
        ctx: EnvironmentCheckContext,
        env: ProjectEnvironment,
        operation: EnvironmentOperation,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let failed_state = state.clone();
            let failed_store = store.clone();
            let failed_op = operation.clone();
            let result = tokio::task::spawn_blocking(move || {
                run_apply(&state, &project, &store, &ctx, &env, &operation)
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                if let Ok(_lock) = failed_store.lock(&failed_op.local_project_id) {
                    if failed_state
                        .owned
                        .session_store
                        .get_project(&failed_op.local_project_id)
                        .is_ok()
                    {
                        if let Ok(mut journal) =
                            failed_store.load_applies(&failed_op.local_project_id)
                        {
                            if let Some(a) = current_attempt(&mut journal, &failed_op) {
                                if let Some(id) = a.active_requirement.take() {
                                    if !a.uncertain_requirements.contains(&id) {
                                        a.uncertain_requirements.push(id);
                                    }
                                }
                                a.operation.phase = EnvironmentOperationPhase::Failed;
                                a.operation.updated_at_ms = crate::session::unix_epoch_ms();
                                a.operation.recovery_state = EnvironmentRecoveryState::Resumable {
                                    checkpoint_digest: metadata_digest(&a.operation.receipts),
                                    last_settled_requirement_id: a
                                        .operation
                                        .receipts
                                        .last()
                                        .and_then(|r| r.requirement_id.clone()),
                                };
                            }
                            let _ =
                                failed_store.save_applies(&failed_op.local_project_id, &journal);
                        }
                    }
                }
            }
        });
    }
}
fn current_attempt<'a>(
    journal: &'a mut EnvironmentApplies,
    op: &EnvironmentOperation,
) -> Option<&'a mut ApplyAttempt> {
    latest_apply_attempt(journal, op)
}
fn cancelled(store: &ProjectEnvironmentStore, op: &EnvironmentOperation) -> bool {
    store
        .load_applies(&op.local_project_id)
        .ok()
        .and_then(|mut j| {
            current_attempt(&mut j, op)
                .map(|a| a.operation.cancellation.is_some() || !apply_active(&a.operation))
        })
        .unwrap_or(true)
}
fn run_apply(
    state: &KernelRuntimeState,
    project: &crate::session::RuntimeProject,
    store: &ProjectEnvironmentStore,
    ctx: &EnvironmentCheckContext,
    env: &ProjectEnvironment,
    op: &EnvironmentOperation,
) -> Result<(), DaemonError> {
    let selected = apply_selection(env, &op.selected_items)?;
    {
        let _lock = store.lock(project.id())?;
        validate_project(state, project, &ctx.owner, env, store)?;
        if current_check_target(&state.owned.config_projection.snapshot()) != op.target
            || cancelled(store, op)
        {
            return Err(environment_error(
                "Apply target changed or operation cancelled",
            ));
        }
        let mut journal = store.load_applies(project.id())?;
        let scopes = current_attempt(&mut journal, op)
            .ok_or_else(|| environment_error("Apply attempt displaced"))?
            .binding_scopes
            .clone();
        retire_removed_apply_bindings(store, env, &scopes)?;
    }
    let mut workspace = CheckWorkspace::default();
    let mut good = std::collections::BTreeSet::new();
    for id in selected {
        let r = environment_requirements(env)
            .find(|r| r.requirement_id == id)
            .expect("validated selection");
        let lock = store.lock(project.id())?;
        validate_project(state, project, &ctx.owner, env, store)?;
        if current_check_target(&state.owned.config_projection.snapshot()) != op.target {
            return Err(environment_error("Apply target generation changed"));
        }
        let mut journal = store.load_applies(project.id())?;
        let a = current_attempt(&mut journal, op)
            .ok_or_else(|| environment_error("Apply attempt displaced"))?;
        if a.operation.cancellation.is_some() {
            drop(lock);
            break;
        }
        a.operation.phase = EnvironmentOperationPhase::Running;
        let uncertain = a.uncertain_requirements.contains(&id);
        let previous_receipt = a
            .operation
            .receipts
            .iter()
            .find(|receipt| receipt.requirement_id.as_deref() == Some(&id))
            .cloned();
        let mut checks = store.load_checks(project.id())?;
        let mut observation = check_requirement_in_workspace(
            ctx,
            env,
            r,
            &op.target,
            &op.operation_id,
            &mut checks.public_bindings,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
            &mut workspace,
        );
        let mut result = apply_item_result(
            r,
            observation.status.clone(),
            &observation.reason_code,
            &observation.safe_summary,
        );
        let deps_ready = environment_dependencies(r).iter().all(|d| good.contains(d));
        let opt_in = op
            .per_item_opt_ins
            .iter()
            .any(|i| i.requirement_id == id && i.decision == EnvironmentOptInDecision::Accepted);
        let mut binding_to_apply = false;
        let mut setup_to_run = false;
        let mut capability_to_apply = false;
        if !deps_ready {
            result = apply_item_result(
                r,
                EnvironmentObservationStatus::Conflict,
                "dependency_blocked",
                "A prerequisite is unready; item was not applied",
            );
        } else if result.status != EnvironmentObservationStatus::Conflict {
            match &r.spec {
                RequirementSpec::Variables { .. } | RequirementSpec::Secrets { .. }
                    if result.status == EnvironmentObservationStatus::Ready =>
                {
                    // Intent persists before the atomic metadata side effect. Repeating
                    // this binding is safe; values are always re-resolved by launch.
                    a.active_requirement = Some(id.clone());
                    store.save_applies(project.id(), &journal)?;
                    binding_to_apply = true;
                }
                RequirementSpec::SetupChecks {
                    commands, probes, ..
                } if !commands.is_empty() => {
                    let mut probe = r.clone();
                    if let RequirementSpec::SetupChecks {
                        commands, legacy, ..
                    } = &mut probe.spec
                    {
                        commands.clear();
                        *legacy = None;
                    }
                    let checked = check_requirement_in_workspace(
                        ctx,
                        env,
                        &probe,
                        &op.target,
                        &op.operation_id,
                        &mut checks.public_bindings,
                        std::time::Instant::now() + std::time::Duration::from_secs(60),
                        &mut workspace,
                    );
                    // Applicability and probe-admission failures cannot authorize side effects.
                    // Only a measured readiness failure can be repaired by opted-in Setup.
                    if !matches!(checked.reason_code.as_str(), "checks_passed" | "check_failed") {
                        observation = checked;
                        result = apply_item_result(r, observation.status.clone(), &observation.reason_code, &observation.safe_summary);
                    } else if setup_reconciliation_allowed(
                        previous_receipt.is_some(), uncertain,
                        crate::runtime::state::project_environment_setup::setup_commands_completed(state, env, r, op),
                        !probes.is_empty() && checked.status == EnvironmentObservationStatus::Ready,
                    )
                    {
                        observation = checked;
                        result = apply_item_result(
                            r,
                            EnvironmentObservationStatus::Ready,
                            "setup_reconciled",
                            "Completed work was rechecked on the target; no command repeated",
                        );
                    } else if previous_receipt.is_some() || (uncertain && !crate::runtime::state::project_environment_setup::setup_retry_is_settled(state, env, r, op)) {
                        result = apply_item_result(r, EnvironmentObservationStatus::NeedsYourInput, "setup_uncertain", "Prior command effects could not be verified; review target facts and make a new plan, no replay performed");
                    } else if !opt_in {
                        result = apply_item_result(
                            r,
                            EnvironmentObservationStatus::NeedsYourInput,
                            "setup_opt_in_required",
                            "This bounded Setup step needs individual opt-in",
                        );
                    } else if probes.is_empty() {
                        result = apply_item_result(
                            r,
                            EnvironmentObservationStatus::NeedsYourInput,
                            "setup_validation_required",
                            "Declare named target checks before running bounded Setup",
                        );
                    } else {
                        a.active_requirement = Some(id.clone());
                        a.operation.phase = EnvironmentOperationPhase::Validating;
                        store.save_applies(project.id(), &journal)?;
                        setup_to_run = true;
                    }
                }
                RequirementSpec::AgentTools { package_digest, .. }
                    if result.status == EnvironmentObservationStatus::Ready =>
                {
                    if package_digest.is_none() {
                        result = apply_item_result(
                            r,
                            EnvironmentObservationStatus::NeedsYourInput,
                            "capability_pin_required",
                            "Pin the reviewed Agent-tool package before scoped Apply",
                        );
                    } else {
                        a.active_requirement = Some(id.clone());
                        store.save_applies(project.id(), &journal)?;
                        capability_to_apply = true;
                    }
                }
                RequirementSpec::CharioxApps { content_digest, .. }
                    if result.status == EnvironmentObservationStatus::Ready =>
                {
                    if content_digest.is_none() {
                        result = apply_item_result(
                            r,
                            EnvironmentObservationStatus::NeedsYourInput,
                            "capability_pin_required",
                            "Pin the admitted App package before scoped Apply",
                        );
                    } else {
                        a.active_requirement = Some(id.clone());
                        store.save_applies(project.id(), &journal)?;
                        capability_to_apply = true;
                    }
                }
                // Existing admitted packages/grants remain their registry's
                // authority. A missing package is not an implicit install/grant.
                _ => {}
            }
        }
        drop(lock);
        if binding_to_apply {
            let budget = std::time::Duration::from_secs(60);
            #[cfg(test)]
            let budget = std::env::var("CHARIOX_APPLY_BINDING_BUDGET_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0 && *v <= 60000)
                .map(std::time::Duration::from_millis)
                .unwrap_or(budget);
            let mut next_poll = std::time::Instant::now();
            let mut stopped = false;
            let prepared = prepare_scoped_binding(
                &ctx.config,
                env,
                r,
                std::time::Instant::now() + budget,
                || {
                    if std::time::Instant::now() >= next_poll {
                        next_poll =
                            std::time::Instant::now() + std::time::Duration::from_millis(25);
                        stopped = cancelled(store, op)
                            || state.authorize_current_external_command().is_err();
                    }
                    stopped
                },
            );
            let lock = store.lock(project.id())?;
            validate_project(state, project, &ctx.owner, env, store)?;
            if current_check_target(&state.owned.config_projection.snapshot()) != op.target {
                return Err(environment_error(
                    "Apply target changed before binding publication",
                ));
            }
            let mut journal = store.load_applies(project.id())?;
            let attempt = current_attempt(&mut journal, op).ok_or_else(|| {
                environment_error("Apply attempt changed before binding publication")
            })?;
            if !apply_active(&attempt.operation)
                || attempt.active_requirement.as_deref() != Some(id.as_str())
            {
                return Err(environment_error("Apply binding intent changed"));
            }
            if attempt.operation.cancellation.is_some() {
                attempt.active_requirement = None;
                store.save_applies(project.id(), &journal)?;
                drop(lock);
                break;
            }
            if prepared
                .and_then(|proof| publish_scoped_binding(store, env, r, proof))
                .is_err()
            {
                result=apply_item_result(r,EnvironmentObservationStatus::NeedsYourInput,"binding_conflict","Scoped binding is unavailable, unproven or conflicts with existing state; review it");
            }
        }
        if capability_to_apply
            && tokio::runtime::Handle::current()
                .block_on(
                    state.apply_project_capability(env, r, &ctx.owner, op, || cancelled(store, op)),
                )
                .is_err()
        {
            result = apply_item_result(
                r,
                EnvironmentObservationStatus::NeedsYourInput,
                "capability_admission_required",
                "Existing capability admission/grant service needs authorization; review it there",
            );
        }
        if setup_to_run {
            let outcome = crate::runtime::state::project_environment_setup::apply_setup_requirement(
                state,
                &ctx.config,
                env,
                r,
                op,
                || cancelled(store, op) || state.authorize_current_external_command().is_err(),
            );
            let mut probe = r.clone();
            if let RequirementSpec::SetupChecks {
                commands, legacy, ..
            } = &mut probe.spec
            {
                commands.clear();
                *legacy = None;
            }
            // Setup may change the folder, so its post-check measures again.
            workspace = CheckWorkspace::default();
            let checked = check_requirement_in_workspace(
                ctx,
                env,
                &probe,
                &op.target,
                &op.operation_id,
                &mut checks.public_bindings,
                std::time::Instant::now() + std::time::Duration::from_secs(60),
                &mut workspace,
            );
            if outcome.is_ok() && checked.status == EnvironmentObservationStatus::Ready {
                observation = checked;
                result = apply_item_result(
                    r,
                    EnvironmentObservationStatus::Ready,
                    "setup_checked",
                    "Bounded Setup completed and named target checks passed",
                );
            } else {
                result = apply_item_result(r, EnvironmentObservationStatus::NeedsYourInput, "setup_unconfirmed", "Setup boundary, command or target validation did not pass; inspect the target before retry");
            }
        }
        let _lock = store.lock(project.id())?;
        validate_project(state, project, &ctx.owner, env, store)?;
        let mut journal = store.load_applies(project.id())?;
        // MP-11: a Check from another client must not be overwritten by the
        // observation snapshot taken before a long worker command.
        let mut latest_checks = store.load_checks(project.id())?;
        for (key, value) in &checks.public_bindings {
            latest_checks
                .public_bindings
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
        let a = current_attempt(&mut journal, op)
            .ok_or_else(|| environment_error("Apply attempt changed before receipt"))?;
        if result.status == EnvironmentObservationStatus::Ready {
            let receipt = previous_receipt.unwrap_or_else(|| EnvironmentReceipt {
                receipt_id: format!("apply-receipt-{:032x}", rand::random::<u128>()),
                requirement_id: Some(id.clone()),
                target: op.target.clone(),
                revision_digest: env.content_digest.clone(),
                artifact_digest: metadata_digest(&r.spec),
                recorded_at_ms: crate::session::unix_epoch_ms(),
            });
            result.receipt_ids = vec![receipt.receipt_id.clone()];
            if !a
                .operation
                .receipts
                .iter()
                .any(|old| old.receipt_id == receipt.receipt_id)
            {
                a.operation.receipts.push(receipt);
            }
            a.uncertain_requirements.retain(|old| old != &id);
            good.insert(id.clone());
        } else if setup_to_run {
            // A nonzero exit/cancellation can still have side effects. Recheck
            // before any subsequent attempt instead of repeating the script.
            if !a.uncertain_requirements.contains(&id) {
                a.uncertain_requirements.push(id.clone());
            }
        }
        a.active_requirement = None;
        observation.status = result.status.clone();
        observation.reason_code = result.reason_code.clone();
        observation.safe_summary = result.safe_summary.clone();
        latest_checks.record_observation(observation);
        latest_checks.folder_binding_digest = folder_bindings_digest(env);
        store.save_checks(project.id(), &latest_checks)?;
        a.operation.per_item_results.push(result);
        a.operation.updated_at_ms = crate::session::unix_epoch_ms();
        a.operation.recovery_state = EnvironmentRecoveryState::Resumable {
            checkpoint_digest: metadata_digest(&a.operation.receipts),
            last_settled_requirement_id: a
                .operation
                .receipts
                .last()
                .and_then(|r| r.requirement_id.clone()),
        };
        store.save_applies(project.id(), &journal)?;
    }
    let _lock = store.lock(project.id())?;
    let mut journal = store.load_applies(project.id())?;
    let a = current_attempt(&mut journal, op)
        .ok_or_else(|| environment_error("Apply attempt changed"))?;
    if !crate::runtime::state::project_environment_setup::apply_setup_cleanup_settled(
        state, env, op,
    ) {
        a.operation.phase = EnvironmentOperationPhase::Failed;
        a.operation.recovery_state = EnvironmentRecoveryState::Resumable {
            checkpoint_digest: metadata_digest(&a.operation.receipts),
            last_settled_requirement_id: a
                .operation
                .receipts
                .last()
                .and_then(|r| r.requirement_id.clone()),
        };
        store.save_applies(project.id(), &journal)?;
        return Ok(());
    }
    if let Some(cancellation) = &mut a.operation.cancellation {
        cancellation.settled_at_ms = Some(crate::session::unix_epoch_ms());
        a.operation.phase = EnvironmentOperationPhase::Cancelled;
    } else if a
        .operation
        .per_item_results
        .iter()
        .all(|r| r.status == EnvironmentObservationStatus::Ready)
    {
        a.operation.phase = EnvironmentOperationPhase::Ready;
    } else {
        a.operation.phase = EnvironmentOperationPhase::AwaitingInput;
    }
    a.operation.updated_at_ms = crate::session::unix_epoch_ms();
    a.operation.recovery_state = EnvironmentRecoveryState::Settled;
    let needs_input = a.operation.phase == EnvironmentOperationPhase::AwaitingInput;
    let operation = a.operation.clone();
    store.save_applies(project.id(), &journal)?;
    drop(_lock);
    if needs_input {
        state.notify_environment_named_inputs(env.clone(), operation, ctx.owner.clone());
    }
    Ok(())
}
