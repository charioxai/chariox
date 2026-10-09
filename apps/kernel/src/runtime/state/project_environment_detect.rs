//! MP-08 / MP-10 / MP-11: Project-authorized Detect; no Save, Check, Apply or grant side effects.
use super::*;
use crate::project_environment::*;

impl KernelRuntimeState {
    pub(crate) async fn detect_project_environment(
        &self,
        request: crate::local::DetectProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let config = self.owned.config_projection.snapshot();
        if request.operation_id.is_empty()
            || request.operation_id.len() > 128
            || request.operation_id.chars().any(char::is_control)
        {
            return Err(environment_error("invalid Detect operation identity"));
        }
        // Detection describes this source kernel; target readiness is never inferred here.
        if request.target.machine_id != config.host_machine_id
            || request.target.target_instance_generation != config.daemon_id
            || request.target.slice_ref.is_some()
        {
            return Err(environment_error(
                "Detect target does not match this source kernel",
            ));
        }
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let mut environment = store.snapshot(&project)?;
        if request
            .folder_ids
            .iter()
            .chain(&request.allow_model_folders)
            .any(|id| !environment.folders.iter().any(|f| &f.folder_id == id))
        {
            return Err(environment_error(
                "Detect folder does not belong to Project",
            ));
        }
        let mut folders: Vec<_> = environment
            .folders
            .iter()
            .filter(|f| request.folder_ids.is_empty() || request.folder_ids.contains(&f.folder_id))
            .cloned()
            .collect();
        if request
            .allow_model_folders
            .iter()
            .any(|id| !folders.iter().any(|f| &f.folder_id == id))
        {
            return Err(environment_error(
                "model opt-in must name a selected folder",
            ));
        }
        for folder in &mut folders {
            let preflight = crate::git_worktree_placement::preflight_working_directory(
                Path::new(&folder.local_workspace_binding),
                "Detect Project environment",
                false,
                &[],
            )?;
            if !Path::new(&folder.local_workspace_binding)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                folder.local_workspace_binding = preflight.canonical_path.to_string_lossy().into();
            }
        }
        let lock_store = store.clone();
        let lock_project = project.id().to_string();
        let _lock = tokio::task::spawn_blocking(move || lock_store.lock(&lock_project))
            .await
            .map_err(|_| environment_error("Detect lock task failed"))??;
        let current = self.owned.session_store.get_project(project.id())?;
        if current.owner_user_id() != caller_user_id
            || current.workspace_ids() != project.workspace_ids()
        {
            return Err(environment_error("Project bindings changed; Detect again"));
        }
        let previous = store.load_detection(project.id())?;
        // Deterministic Detect covers every Project folder; selection limits provider disclosure.
        let mut roots = environment.folders.clone();
        for root in &mut roots {
            let preflight = crate::git_worktree_placement::preflight_working_directory(
                Path::new(&root.local_workspace_binding),
                "Detect Project environment",
                false,
                &[],
            )?;
            if !Path::new(&root.local_workspace_binding)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                root.local_workspace_binding = preflight.canonical_path.to_string_lossy().into();
            }
        }
        let id = environment.lineage.environment_id.clone();
        let detection = tokio::task::spawn_blocking(move || detect_environment(&roots, &id))
            .await
            .map_err(|_| environment_error("Detect index task failed"))??;
        let model_folders =
            detection.model_folders(previous.as_ref(), &request.allow_model_folders, &folders);
        let now = crate::session::unix_epoch_ms();
        let mut results = detection.skips.clone();
        if model_folders.is_empty() {
            results.push(EnvironmentItemResult {
                requirement_id: "detect:model".into(),
                status: EnvironmentObservationStatus::NeedsYourInput,
                reason_code: if previous
                    .as_ref()
                    .is_some_and(|p| p.evidence_digest == detection.evidence_digest)
                {
                    "unchanged_evidence"
                } else {
                    "deterministic_only"
                }
                .into(),
                safe_summary: if previous
                    .as_ref()
                    .is_some_and(|p| p.evidence_digest == detection.evidence_digest)
                {
                    "Unchanged evidence · utility skipped"
                } else {
                    "Deterministic detection · no provider run"
                }
                .into(),
                receipt_ids: vec![],
            });
        } else {
            let (input, metadata_limited) =
                detection.discovery_input(project.id(), &folders, &model_folders);
            if metadata_limited {
                results.push(EnvironmentItemResult {
                    requirement_id: "detect:model-metadata".into(),
                    status: EnvironmentObservationStatus::NeedsYourInput,
                    reason_code: "utility_metadata_bounded".into(),
                    safe_summary: "Utility metadata limited to 8 file names per folder, 32 reference names and one existing use site per name (32 KiB total). Full deterministic proposals and provenance are retained.".into(),
                    receipt_ids: vec![],
                });
            }
            let primary = folders
                .iter()
                .find(|f| model_folders.contains(&f.folder_id))
                .expect("model folder selected");
            // Normal official utility path; scratch metadata-only view, no provider-native tools.
            let utility_result = self
                .detect_environment_utility(&project, primary, request.provider.as_ref(), &input)
                .await;
            let error_text = utility_result
                .as_ref()
                .err()
                .map(|error| error.to_string().to_ascii_lowercase())
                .unwrap_or_default();
            let unauthorized = error_text.contains("401") || error_text.contains("unauthorized");
            if let Err(error) = &utility_result {
                // Public phase/reason identifiers only: never log provider payloads or credentials.
                let phase = match error {
                    DaemonError::LocalTransport { operation, .. }
                    | DaemonError::ProviderProtocol { operation, .. } => *operation,
                    _ => "provider utility",
                };
                let reason = if unauthorized {
                    "provider_unauthorized"
                } else if error_text.contains("emfile")
                    || error_text.contains("too many open files")
                {
                    "resource_open_files"
                } else if error_text.contains("not running") {
                    "provider_not_ready"
                } else if error_text.contains("timeout")
                    || error_text.contains("timed out")
                    || error_text.contains("did not complete within")
                {
                    "provider_timeout"
                } else if error_text.contains("model")
                    && (error_text.contains("not supported") || error_text.contains("unsupported"))
                {
                    "unsupported_model"
                } else if error_text.contains("exceeds bounds") {
                    "metadata_bounds"
                } else {
                    "utility_failed"
                };
                crate::logging::warn_with_fields(
                    "daemon.environment_detect",
                    "provider detection utility failed",
                    serde_json::json!({ "phase": phase, "reason": reason }),
                );
            }
            results.push(EnvironmentItemResult { requirement_id: "detect:model".into(), status: EnvironmentObservationStatus::NeedsYourInput,
                reason_code: if utility_result.is_ok() { "utility_completed" } else if unauthorized { "provider_unauthorized" } else { "utility_failed" }.into(),
                safe_summary: if utility_result.is_ok() { "Official provider metadata utility completed · proposals need review" } else if unauthorized { "Provider returned unauthorized/401 · check Provider Accounts; deterministic proposals retained" } else { "Provider utility failed · deterministic proposals retained; check provider sign-in and retry" }.into(), receipt_ids: vec![] });
        }
        let operation = EnvironmentOperation {
            operation_id: request.operation_id,
            attempt: 1,
            local_project_id: project.id().into(),
            revision_digest: environment.content_digest.clone(),
            target: request.target,
            kind: EnvironmentOperationKind::Detect,
            phase: if results.iter().any(|r| {
                r.reason_code == "utility_failed" || r.reason_code == "provider_unauthorized"
            }) {
                EnvironmentOperationPhase::Failed
            } else {
                EnvironmentOperationPhase::Ready
            },
            selected_items: folders.iter().map(|f| f.folder_id.clone()).collect(),
            per_item_opt_ins: vec![],
            per_item_results: results,
            created_at_ms: now,
            updated_at_ms: crate::session::unix_epoch_ms(),
            cancellation: None,
            receipts: vec![],
            recovery_state: EnvironmentRecoveryState::Settled,
        };
        let current = self.owned.session_store.get_project(project.id())?;
        if current.owner_user_id() != caller_user_id
            || current.workspace_ids() != project.workspace_ids()
        {
            return Err(environment_error("Project bindings changed; Detect again"));
        }
        store.save_detection(&EnvironmentDetectionCache {
            project_id: project.id().into(),
            evidence_digest: detection.evidence_digest.clone(),
            proposals: detection.proposals.clone(),
            operation: operation.clone(),
        })?;
        environment.proposals.retain(|p| !matches!(&p.requirement.scope, RequirementScope::Folder { folder_id } if environment.folders.iter().any(|f| &f.folder_id == folder_id)));
        environment.proposals.extend(detection.proposals);
        environment.evidence_digest = Some(detection.evidence_digest);
        environment.operations = vec![operation];
        refresh_environment_content_digest(&mut environment);
        Ok(LocalDaemonResponse::ProjectEnvironment { environment })
    }
}
