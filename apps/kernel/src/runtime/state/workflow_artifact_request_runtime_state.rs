use super::workflow_code_request_support::*;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_code_artifact_create_request(
        &self,
        request: crate::local::CreateWorkflowCodeArtifactRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = workflow_code_artifact_actor(caller_user_id, caller_metaagent_id);
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let schema_import_root =
                workflow_code_schema_import_root_for_session(app, &request.session_id)?;
            let compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &request.node_path,
                    &request.source,
                    request.language,
                    &limits,
                    schema_import_root.as_deref(),
                )?;
            reject_invalid_workflow_code_artifact_validation(
                "workflow_code_artifact.create",
                &compile.validation,
            )?;
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.save(
                &request.name,
                request.language,
                request.source,
                compile.definition,
                compile.validation,
                actor,
                crate::workflow_code::WorkflowCodeArtifactHistoryAction::Created,
            )?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.created",
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_update_request(
        &self,
        request: crate::local::UpdateWorkflowCodeArtifactRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = workflow_code_artifact_actor(caller_user_id, caller_metaagent_id);
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let schema_import_root =
                workflow_code_schema_import_root_for_session(app, &request.session_id)?;
            let compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    &request.node_path,
                    &request.source,
                    request.language,
                    &limits,
                    schema_import_root.as_deref(),
                )?;
            reject_invalid_workflow_code_artifact_validation(
                "workflow_code_artifact.update",
                &compile.validation,
            )?;
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.update(
                &request.name,
                request.language,
                request.source,
                compile.definition,
                compile.validation,
                actor,
                crate::workflow_code::WorkflowCodeArtifactHistoryAction::Updated,
            )?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.updated",
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_source_bind_request(
        &self,
        request: crate::local::BindWorkflowCodeSourceRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.get(&request.artifact_name)?.ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.bind",
                    message: format!(
                        "workflow-code artifact `{}` is not saved",
                        request.artifact_name
                    ),
                }
            })?;
            let bindings = workflow_code_bindings_for_existing_workflow(
                app,
                &request.session_id,
                &request.workflow_ref,
                &artifact.definition,
            )?;
            let workflow = app.sessions_mut().bind_workflow_code_source(
                &request.session_id,
                &request.workflow_ref,
                request.expected_workflow_revision,
                artifact.metadata.name,
                artifact.metadata.language,
                artifact.metadata.source_sha256,
                request.origin,
                bindings,
            )?;
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&request.session_id)?;
            app.durable_state_store().append_event(
                "workflow_code_source.bound",
                Some(request.session_id),
                serde_json::json!({ "workflow": &workflow }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeSourceBound { workflow, session })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_get_request(
        &self,
        request: crate::local::GetWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact =
                registry
                    .get(&request.name)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.get",
                        message: format!("workflow-code artifact `{}` is not saved", request.name),
                    })?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifact { artifact })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_list_request(
        &self,
        request: crate::local::ListWorkflowCodeArtifactsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifacts = registry.list()?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_delete_request(
        &self,
        request: crate::local::DeleteWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let path = registry.delete(&request.name)?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.deleted",
                Some(request.session_id),
                serde_json::json!({
                    "name": &request.name,
                    "path": &path,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactDeleted {
                name: request.name,
                path,
            })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_export_request(
        &self,
        request: crate::local::ExportWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.export_workflow_code_package_response(
            request.session_id,
            request.name,
            None,
            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            "workflow_code_artifact.exported",
            true,
        )
        .await
    }

    pub(super) async fn execute_workflow_code_package_export_request(
        &self,
        request: crate::local::ExportWorkflowCodePackageRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.export_workflow_code_package_response(
            request.session_id,
            request.name,
            request.target,
            request.agent_mode,
            "workflow_code_package.exported",
            false,
        )
        .await
    }

    pub(super) async fn export_workflow_code_package_response(
        &self,
        session_id: String,
        name: String,
        target: Option<crate::local::WorkflowCodePackageExportTarget>,
        agent_mode: crate::workflow_code::WorkflowCodeSourceExportAgentMode,
        event_kind: &'static str,
        legacy_response: bool,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let package = match target {
                None => {
                    let registry = workflow_code_registry_for_session(app, &session_id)?;
                    registry.export_package(&name)?
                }
                Some(crate::local::WorkflowCodePackageExportTarget::Artifact {
                    name: artifact_name,
                }) => {
                    let registry = workflow_code_registry_for_session(app, &session_id)?;
                    registry.export_package(&artifact_name)?
                }
                Some(crate::local::WorkflowCodePackageExportTarget::Workflow { workflow_ref }) => {
                    let session = crate::app::KernelSessionReadService::new(app)
                        .session_snapshot(&session_id)?;
                    crate::workflow_code::export_workflow_code_package_from_session_workflow(
                        &session,
                        &workflow_ref,
                        &name,
                        agent_mode,
                    )?
                }
            };
            app.durable_state_store().append_event(
                event_kind,
                Some(session_id),
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            )?;
            if legacy_response {
                Ok(LocalDaemonResponse::WorkflowCodeArtifactExported { package })
            } else {
                Ok(LocalDaemonResponse::WorkflowCodePackageExported { package })
            }
        })
        .await
    }

    pub(super) async fn execute_workflow_code_artifact_import_request(
        &self,
        request: crate::local::ImportWorkflowCodeArtifactRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.import_workflow_code_package_response(
            request,
            caller_user_id,
            caller_metaagent_id,
            "workflow_code_artifact.imported",
            true,
        )
        .await
    }

    pub(super) async fn execute_workflow_code_package_import_request(
        &self,
        request: crate::local::ImportWorkflowCodePackageRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.import_workflow_code_package_response(
            request,
            caller_user_id,
            caller_metaagent_id,
            "workflow_code_package.imported",
            false,
        )
        .await
    }

    pub(super) async fn import_workflow_code_package_response(
        &self,
        request: crate::local::ImportWorkflowCodePackageRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
        event_kind: &'static str,
        legacy_response: bool,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = workflow_code_artifact_actor(caller_user_id, caller_metaagent_id);
        self.with_app_side_effect(move |app| {
            request.package.validate_integrity()?;
            let limits = app.config().workflow_code_limits();
            let validation = request.package.definition.validate_with_limits(&limits);
            reject_invalid_workflow_code_artifact_validation(
                "workflow_code_artifact.import",
                &validation,
            )?;
            let definition = request.package.definition.clone();
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.import_package(
                request.name.as_deref(),
                request.package,
                definition,
                validation,
                actor,
                request.overwrite,
            )?;
            app.durable_state_store().append_event(
                event_kind,
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                    "overwrite": request.overwrite,
                }),
            )?;
            if legacy_response {
                Ok(LocalDaemonResponse::WorkflowCodeArtifactImported { artifact })
            } else {
                Ok(LocalDaemonResponse::WorkflowCodePackageImported { artifact })
            }
        })
        .await
    }

    pub(super) async fn execute_workflow_code_source_export_request(
        &self,
        request: crate::local::ExportWorkflowCodeSourceRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let export = match request.target {
                crate::local::WorkflowCodeSourceExportTarget::Artifact { name } => {
                    if request.agent_mode
                        != crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.source_export",
                            message: "agent_mode is only supported when exporting an existing workflow"
                                .to_string(),
                        });
                    }
                    registry.export_source(&name, request.format)?
                }
                crate::local::WorkflowCodeSourceExportTarget::Workflow { workflow_ref } => {
                    let session = crate::app::KernelSessionReadService::new(app)
                        .session_snapshot(&request.session_id)?;
                    crate::workflow_code::export_workflow_code_source_from_session_workflow(
                        &session,
                        &workflow_ref,
                        request.format,
                        request.agent_mode,
                    )?
                }
            };
            app.durable_state_store().append_event(
                "workflow_code_source.exported",
                Some(request.session_id),
                serde_json::json!({
                    "name": &export.name,
                    "format": export.format,
                    "source_path": &export.source_path,
                    "source_sha256": &export.source_sha256,
                    "source_bytes": export.source_bytes,
                    "definition_sha256": &export.definition_sha256,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeSourceExported { export })
        })
        .await
    }
}
