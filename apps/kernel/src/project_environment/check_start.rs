//! MP-08 / MP-10 / MP-11: durable Check admission belongs on the blocking I/O pool.
use super::*;

pub(crate) enum CheckStart {
    Replay(EnvironmentOperation),
    New {
        environment: ProjectEnvironment,
        operation: EnvironmentOperation,
        workspace_ids: Vec<String>,
    },
}
impl ProjectEnvironmentStore {
    pub(crate) fn start_check(
        &self,
        request: crate::local::CheckProjectEnvironmentRequest,
        user: &str,
        target: EnvironmentTargetBinding,
        validate: impl FnOnce() -> Result<crate::session::RuntimeProject, crate::error::DaemonError>,
    ) -> Result<CheckStart, crate::error::DaemonError> {
        let started = std::time::Instant::now();
        let lock = self.lock_briefly(&request.project_id)?;
        let lock_us = started.elapsed().as_micros();
        let phase = std::time::Instant::now();
        let project = validate()?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let env = self.snapshot_locked(&project)?;
        let snapshot_us = phase.elapsed().as_micros();
        let phase = std::time::Instant::now();
        if env.content_digest != request.revision_digest {
            return Err(environment_error("revision conflict; refresh before Check"));
        }
        let requirements: Vec<_> = env
            .project_requirements
            .iter()
            .chain(env.folders.iter().flat_map(|f| &f.requirements))
            .collect();
        if request
            .selected_items
            .iter()
            .any(|id| !requirements.iter().any(|r| &r.requirement_id == id))
        {
            return Err(environment_error(
                "Check selection does not belong to the saved revision",
            ));
        }
        let mut checks = self.load_checks(project.id())?;
        checks.observations.retain(|o| {
            requirements
                .iter()
                .any(|r| r.requirement_id == o.requirement_id)
        });
        if checks.folder_binding_digest != folder_bindings_digest(&env) {
            for observation in &mut checks.observations {
                observation.stale = true;
            }
        }

        for op in &mut checks.operations {
            if op.target != target
                && matches!(
                    op.phase,
                    EnvironmentOperationPhase::Requested
                        | EnvironmentOperationPhase::Running
                        | EnvironmentOperationPhase::Validating
                )
            {
                op.phase = EnvironmentOperationPhase::Failed;
                op.recovery_state = EnvironmentRecoveryState::Settled;
            }
        }
        if let Some(op) = checks
            .operations
            .iter()
            .find(|o| o.operation_id == request.operation_id)
        {
            if op.revision_digest != request.revision_digest
                || op.target != target
                || op.selected_items != request.selected_items
            {
                return Err(environment_error(
                    "Check operation identity already names another request",
                ));
            }
            return Ok(CheckStart::Replay(op.clone()));
        }
        if checks.operations.iter().any(|o| {
            o.target == target
                && matches!(
                    o.phase,
                    EnvironmentOperationPhase::Requested
                        | EnvironmentOperationPhase::Running
                        | EnvironmentOperationPhase::Validating
                )
        }) {
            return Err(environment_error(
                "Check already running for this Project; refresh its operation",
            ));
        }
        let now = crate::session::unix_epoch_ms();
        let operation = EnvironmentOperation {
            operation_id: request.operation_id.clone(),
            attempt: 1,
            local_project_id: project.id().into(),
            revision_digest: env.content_digest.clone(),
            target: target.clone(),
            kind: EnvironmentOperationKind::Check,
            phase: EnvironmentOperationPhase::Requested,
            selected_items: request.selected_items.clone(),
            per_item_opt_ins: vec![],
            per_item_results: vec![],
            created_at_ms: now,
            updated_at_ms: now,
            cancellation: None,
            receipts: vec![],
            recovery_state: EnvironmentRecoveryState::Pending,
        };
        checks.operations.retain(|o| {
            matches!(
                o.phase,
                EnvironmentOperationPhase::Ready
                    | EnvironmentOperationPhase::Failed
                    | EnvironmentOperationPhase::Cancelled
            )
        });
        if checks.operations.len() >= 16 {
            checks.operations.remove(0);
        }
        checks.operations.push(operation.clone());
        let preparation_us = phase.elapsed().as_micros();
        let phase = std::time::Instant::now();
        self.save_checks(project.id(), &checks)?;
        crate::logging::info_with_fields(
            "daemon.environment_check_latency",
            "Check admission persisted",
            serde_json::json!({"lock_us":lock_us,"snapshot_us":snapshot_us,"preparation_us":preparation_us,"write_us":phase.elapsed().as_micros(),"total_us":started.elapsed().as_micros()}),
        );
        drop(lock);
        let workspaces = project.workspace_ids().to_vec();
        Ok(CheckStart::New {
            environment: env,
            operation,
            workspace_ids: workspaces,
        })
    }
}
