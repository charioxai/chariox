//! MP-08 / MP-10 / MP-11: bounded read-only Check execution and observation settlement.
use super::*;
impl ProjectEnvironmentStore {
    pub(crate) fn run_check(
        &self,
        ctx: EnvironmentCheckContext,
        env: ProjectEnvironment,
        operation: EnvironmentOperation,
        validate: impl Fn() -> Result<crate::session::RuntimeProject, crate::error::DaemonError>,
    ) -> Result<(), crate::error::DaemonError> {
        self.run_check_until(
            ctx,
            env,
            operation,
            validate,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        )
    }
    // MP-08 / MP-10 / MP-11: one absolute budget for the whole operation.
    pub(super) fn run_check_until(
        &self,
        ctx: EnvironmentCheckContext,
        env: ProjectEnvironment,
        mut operation: EnvironmentOperation,
        validate: impl Fn() -> Result<crate::session::RuntimeProject, crate::error::DaemonError>,
        deadline: std::time::Instant,
    ) -> Result<(), crate::error::DaemonError> {
        let target = operation.target.clone();
        let lock = self.lock_until(&env.local_project_id, deadline)?;
        let project = validate()?;

        let current = self.snapshot_until(&project, deadline)?;
        if current.content_digest != env.content_digest {
            return Err(environment_error("revision changed before Check"));
        }
        let mut checks = self.load_checks(project.id())?;
        checks.invalidate_observations(&env, &target, crate::session::unix_epoch_ms());
        operation.phase = EnvironmentOperationPhase::Running;
        operation.updated_at_ms = crate::session::unix_epoch_ms();
        if let Some(op) = checks
            .operations
            .iter_mut()
            .find(|o| o.operation_id == operation.operation_id)
        {
            *op = operation.clone();
        }
        self.save_checks(project.id(), &checks)?;
        drop(lock);
        let mut last_progress = std::time::Instant::now();
        for r in env
            .project_requirements
            .iter()
            .chain(env.folders.iter().flat_map(|f| &f.requirements))
            .filter(|r| {
                if operation.selected_items.is_empty() {
                    r.required
                } else {
                    operation.selected_items.contains(&r.requirement_id)
                }
            })
        {
            let observation = if std::time::Instant::now() >= deadline {
                MachineObservation {
                    environment_id: env.lineage.environment_id.clone(),
                    requirement_id: r.requirement_id.clone(),
                    target: target.clone(),
                    revision_digest: env.content_digest.clone(),
                    status: EnvironmentObservationStatus::NeedsYourInput,
                    checked_at_ms: Some(crate::session::unix_epoch_ms()),
                    measured_facts: empty_environment_facts(),
                    reason_code: "check_budget_exceeded".into(),
                    safe_summary:
                        "Check reached its bounded probe budget; select fewer items and check again"
                            .into(),
                    check_operation_id: Some(operation.operation_id.clone()),
                    stale: true,
                }
            } else {
                check_requirement(
                    &ctx,
                    &env,
                    r,
                    &target,
                    &operation.operation_id,
                    &mut checks.public_bindings,
                    deadline,
                )
            };
            let _lock = self.lock_until(&env.local_project_id, deadline)?;
            let project = validate()?;
            if self.snapshot_until(&project, deadline)?.content_digest != env.content_digest {
                return Err(environment_error(
                    "Project changed during Check; check the current revision again",
                ));
            }
            operation.per_item_results.push(EnvironmentItemResult {
                requirement_id: r.requirement_id.clone(),
                status: observation.status.clone(),
                reason_code: observation.reason_code.clone(),
                safe_summary: observation.safe_summary.clone(),
                receipt_ids: vec![],
            });
            let changed_evidence = checks
                .observations
                .iter()
                .find(|prior| {
                    prior.requirement_id == observation.requirement_id
                        && prior.target == observation.target
                })
                .is_none_or(|prior| {
                    prior.status != observation.status
                        || prior.stale != observation.stale
                        || prior.reason_code != observation.reason_code
                        || prior.measured_facts != observation.measured_facts
                        || prior.revision_digest != observation.revision_digest
                });
            checks.record_observation(observation);
            operation.updated_at_ms = crate::session::unix_epoch_ms();
            if let Some(op) = checks
                .operations
                .iter_mut()
                .find(|o| o.operation_id == operation.operation_id)
            {
                *op = operation.clone();
            }
            checks.folder_binding_digest = folder_bindings_digest(&env);
            // MP-08 / MP-10 / MP-11: cheap probes complete in a burst. Keep
            // bounded progress without a durable file/directory sync per item.
            // Changed evidence is visible before the next possibly slow probe.
            // Requested, Running and final settlement remain durable fences.
            if changed_evidence || last_progress.elapsed() >= std::time::Duration::from_millis(100)
            {
                self.save_checks(project.id(), &checks)?;
                last_progress = std::time::Instant::now();
            }
        }
        let _lock = self.lock_until(&env.local_project_id, deadline)?;
        let project = validate()?;
        if self.snapshot_until(&project, deadline)?.content_digest != env.content_digest {
            return Err(environment_error("Project changed during Check"));
        }
        // A dependent cannot become Ready while its prerequisite is missing, stale or excluded.
        checks.invalidate_observations(&env, &target, crate::session::unix_epoch_ms());
        invalidate_check_dependencies(
            &env,
            &mut checks.observations,
            &target,
            Some(&operation.operation_id),
        );
        for result in &mut operation.per_item_results {
            if let Some(o) = checks.observations.iter().find(|o| {
                o.requirement_id == result.requirement_id
                    && o.check_operation_id.as_deref() == Some(&operation.operation_id)
            }) {
                result.status = o.status.clone();
                result.reason_code = o.reason_code.clone();
                result.safe_summary = o.safe_summary.clone();
            }
        }
        operation.phase = EnvironmentOperationPhase::Ready;
        operation.updated_at_ms = crate::session::unix_epoch_ms();
        operation.recovery_state = EnvironmentRecoveryState::Settled;
        if let Some(op) = checks
            .operations
            .iter_mut()
            .find(|o| o.operation_id == operation.operation_id)
        {
            *op = operation;
        }
        checks
            .public_bindings
            .retain(|k, _| k.starts_with(&format!("{}:", env.content_digest)));
        self.save_checks(project.id(), &checks)
    }
}
