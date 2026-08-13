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

    pub(super) async fn execute_workflow_code_source_rebuild_request(
        &self,
        request: crate::local::RebuildWorkflowCodeSourceRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            if workflow.revision() != request.expected_workflow_revision {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: format!(
                        "workflow revision conflict: expected {}, current {}",
                        request.expected_workflow_revision,
                        workflow.revision()
                    ),
                });
            }
            let binding = workflow
                .code_source()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: "workflow does not have a stored code source".to_string(),
                })?;
            let origin = binding.origin();
            let missing_agent_ids = binding
                .bindings()
                .agent_ids
                .values()
                .filter(|agent_id| app.agents().get_agent(agent_id).is_err())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            if !missing_agent_ids.is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: format!(
                        "stored source refers to missing workflow agents: {}",
                        missing_agent_ids.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                });
            }
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.get(binding.artifact_name())?.ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: "stored workflow-code artifact is missing".to_string(),
                }
            })?;
            let source_sha256 = crate::workflow_code::sha256_hex(artifact.source.as_bytes());
            if source_sha256 != binding.source_sha256()
                || source_sha256 != artifact.metadata.source_sha256
            {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: "stored workflow source failed its integrity check".to_string(),
                });
            }
            let limits = app.config().workflow_code_limits();
            let schema_import_root =
                workflow_code_schema_import_root_for_session(app, &request.session_id)?;
            let compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    "node",
                    &artifact.source,
                    artifact.metadata.language,
                    &limits,
                    schema_import_root.as_deref(),
                )?;
            reject_invalid_workflow_code_artifact_validation(
                "workflow_code.rebuild",
                &compile.validation,
            )?;
            let changes = workflow_code_rebuild_structural_changes(
                app,
                &request.session_id,
                &workflow,
                binding.bindings(),
            )?;
            let preview = crate::workflow_code::WorkflowCodeRebuildPreview {
                workflow_id: workflow.id().to_string(),
                current_workflow_revision: workflow.revision(),
                source_workflow_revision: binding.workflow_revision(),
                source_sha256: binding.source_sha256().to_string(),
                diverged: workflow.revision() != binding.workflow_revision(),
                restored_schemas: compile.definition.schemas.len(),
                restored_nodes: compile.definition.nodes.len(),
                restored_edges: compile.definition.edges.len(),
                restored_endpoints: compile.definition.endpoints.len(),
                restored_queues: compile.definition.queues.len(),
                restored_schedules: compile.definition.schedules.len(),
                changes,
            };
            if !request.confirm {
                return Ok(LocalDaemonResponse::WorkflowCodeRebuildPreview { preview });
            }
            let result = app.sessions_mut().rebuild_workflow_code_definition(
                &request.session_id,
                &request.workflow_ref,
                request.expected_workflow_revision,
                &compile.definition,
                artifact.metadata.name,
                artifact.metadata.language,
                artifact.metadata.source_sha256,
                origin,
            )?;
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&request.session_id)?;
            app.durable_state_store().append_event(
                "workflow_code_source.rebuilt",
                Some(request.session_id),
                serde_json::json!({ "preview": &preview, "result": &result }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeSourceRebuilt {
                preview,
                result,
                session,
            })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_source_update_from_workflow_request(
        &self,
        request: crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = workflow_code_artifact_actor(caller_user_id, caller_metaagent_id);
        self.with_app_side_effect(move |app| {
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            if workflow.revision() != request.expected_workflow_revision {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.update_from_workflow",
                    message: format!(
                        "workflow revision conflict: expected {}, current {}",
                        request.expected_workflow_revision,
                        workflow.revision()
                    ),
                });
            }
            let binding = workflow
                .code_source()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "workflow_code.update_from_workflow",
                    message: "workflow does not have a stored code source".to_string(),
                })?;
            let artifact_name = binding.artifact_name().to_string();
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact =
                registry
                    .get(&artifact_name)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.update_from_workflow",
                        message: "stored workflow-code artifact is missing".to_string(),
                    })?;
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&request.session_id)?;
            let export = crate::workflow_code::export_workflow_code_source_from_session_workflow(
                &session,
                workflow.id(),
                crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
            )?;
            let (added_lines, removed_lines) =
                workflow_code_source_changed_line_counts(&artifact.source, &export.source);
            let preview = crate::workflow_code::WorkflowCodeSourceUpdatePreview {
                workflow_id: workflow.id().to_string(),
                workflow_revision: workflow.revision(),
                previous_source_sha256: artifact.metadata.source_sha256.clone(),
                generated_source_sha256: export.source_sha256.clone(),
                changed: artifact.metadata.source_sha256 != export.source_sha256,
                previous_line_count: artifact.source.lines().count(),
                generated_line_count: export.source.lines().count(),
                added_lines,
                removed_lines,
                generated_source: export.source.clone(),
            };
            if !request.confirm {
                return Ok(LocalDaemonResponse::WorkflowCodeSourceUpdatePreview { preview });
            }
            if request.expected_generated_source_sha256.as_deref()
                != Some(export.source_sha256.as_str())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.update_from_workflow",
                    message: "generated workflow source changed after preview; preview it again"
                        .to_string(),
                });
            }
            if !preview.changed {
                return Ok(LocalDaemonResponse::WorkflowCodeSourceUpdated {
                    preview,
                    workflow,
                    session,
                });
            }
            let limits = app.config().workflow_code_limits();
            let schema_import_root =
                workflow_code_schema_import_root_for_session(app, &request.session_id)?;
            let compile =
                crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                    "node",
                    &export.source,
                    export.language,
                    &limits,
                    schema_import_root.as_deref(),
                )?;
            reject_invalid_workflow_code_artifact_validation(
                "workflow_code.update_from_workflow",
                &compile.validation,
            )?;
            let mappings = workflow_code_bindings_for_existing_workflow(
                app,
                &request.session_id,
                workflow.id(),
                &compile.definition,
            )?;
            let artifact = registry.update(
                &artifact_name,
                export.language,
                export.source,
                compile.definition,
                compile.validation,
                actor,
                crate::workflow_code::WorkflowCodeArtifactHistoryAction::Updated,
            )?;
            let workflow = app.sessions_mut().bind_workflow_code_source(
                &request.session_id,
                workflow.id(),
                Some(request.expected_workflow_revision),
                artifact_name,
                artifact.metadata.language,
                artifact.metadata.source_sha256,
                crate::session::WorkflowCodeSourceOrigin::Generated,
                mappings,
            )?;
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&request.session_id)?;
            app.durable_state_store().append_event(
                "workflow_code_source.updated_from_workflow",
                Some(request.session_id),
                serde_json::json!({ "preview": &preview, "workflow": &workflow }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeSourceUpdated {
                preview,
                workflow,
                session,
            })
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

fn workflow_code_rebuild_structural_changes(
    app: &crate::app::DaemonApp,
    session_id: &str,
    workflow: &crate::session::WorkflowDefinition,
    bindings: &crate::workflow_code::WorkflowCodeApplyReport,
) -> Result<Vec<crate::workflow_code::WorkflowCodeStructuralChange>, DaemonError> {
    fn change(
        resource: &str,
        current: impl IntoIterator<Item = String>,
        source: impl IntoIterator<Item = String>,
    ) -> crate::workflow_code::WorkflowCodeStructuralChange {
        let current = current
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let source = source
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        crate::workflow_code::WorkflowCodeStructuralChange {
            resource: resource.to_string(),
            current_count: current.len(),
            source_count: source.len(),
            restore_missing: source.difference(&current).count(),
            remove_visual_only: current.difference(&source).count(),
            replace_existing: source.intersection(&current).count(),
        }
    }

    let session = app.sessions().get_session(session_id)?;
    Ok(vec![
        change(
            "schemas",
            workflow
                .schemas()
                .iter()
                .map(|value| value.id().to_string()),
            bindings.schema_refs.values().cloned(),
        ),
        change(
            "nodes",
            workflow.nodes().iter().map(|value| value.id().to_string()),
            bindings.node_ids.values().cloned(),
        ),
        change(
            "edges",
            workflow.edges().iter().map(|value| value.id().to_string()),
            bindings.edge_ids.values().cloned(),
        ),
        change(
            "endpoints",
            workflow
                .endpoints()
                .iter()
                .map(|value| value.id().to_string()),
            bindings.endpoint_ids.values().cloned(),
        ),
        change(
            "queues",
            session
                .workflow_prompt_queues_for_workflow(workflow.id())
                .into_iter()
                .map(|value| value.id().to_string()),
            bindings.queue_ids.values().cloned(),
        ),
        change(
            "schedules",
            session
                .workflow_schedules()
                .iter()
                .filter(|value| value.workflow_id() == workflow.id())
                .map(|value| value.id().to_string()),
            bindings.schedule_ids.values().cloned(),
        ),
    ])
}

fn workflow_code_source_changed_line_counts(previous: &str, generated: &str) -> (usize, usize) {
    let previous = previous.lines().collect::<Vec<_>>();
    let generated = generated.lines().collect::<Vec<_>>();
    let prefix = previous
        .iter()
        .zip(&generated)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = previous[prefix..]
        .iter()
        .rev()
        .zip(generated[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    (
        generated.len().saturating_sub(prefix + suffix),
        previous.len().saturating_sub(prefix + suffix),
    )
}
