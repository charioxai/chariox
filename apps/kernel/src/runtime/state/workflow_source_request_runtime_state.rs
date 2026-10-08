//! MP-11 F7 / review R1: stored workflow source synchronization through the shared compiler lane.
use super::workflow_code_request_support::*;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_code_source_rebuild_request(
        &self,
        request: crate::local::RebuildWorkflowCodeSourceRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = request.session_id.clone();
        let workflow_ref = request.workflow_ref.clone();
        let expected_workflow_revision = request.expected_workflow_revision;
        let (workflow, artifact, origin) = self
            .with_authorized_app_side_effect({
                let request = request.clone();
                move |app| {
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
                    let binding =
                        workflow
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
                    let source_sha256 =
                        crate::workflow_code::sha256_hex(artifact.source.as_bytes());
                    if source_sha256 != binding.source_sha256()
                        || source_sha256 != artifact.metadata.source_sha256
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.rebuild",
                            message: "stored workflow source failed its integrity check"
                                .to_string(),
                        });
                    }
                    Ok((workflow, artifact, origin))
                }
            })
            .await?;
        let compile = self
            .compile_workflow_request_source(
                &request.session_id,
                &artifact.source,
                artifact.metadata.language,
            )
            .await?;
        let (preview, confirmed_rebuild) = self
            .with_authorized_app_side_effect(move |app| {
                let current = app
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let registry = workflow_code_registry_for_session(app, &request.session_id)?;
                if current != workflow
                    || registry
                        .get(&artifact.metadata.name)?
                        .map(|value| value.metadata)
                        .as_ref()
                        != Some(&artifact.metadata)
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "workflow_code.rebuild",
                        message: "workflow or stored source changed during compilation; retry"
                            .into(),
                    });
                }
                let binding =
                    workflow
                        .code_source()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "workflow_code.rebuild",
                            message: "workflow does not have a stored code source".into(),
                        })?;
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
                    return Ok((preview, None));
                }
                Ok((
                    preview,
                    Some((
                        compile.definition,
                        crate::session::WorkflowCodeSourceDescriptor {
                            artifact_name: artifact.metadata.name,
                            language: artifact.metadata.language,
                            source_sha256: artifact.metadata.source_sha256,
                            origin,
                        },
                    )),
                ))
            })
            .await?;
        let Some((definition, source)) = confirmed_rebuild else {
            return Ok(LocalDaemonResponse::WorkflowCodeRebuildPreview { preview });
        };

        // Compilation, validation, and preview construction above are non-mutating and may run
        // external tooling. Enter the synchronous activity boundary only for the confirmed
        // authoritative rebuild, its durable workflow snapshot, or its in-lock rollback.
        self.authorize_current_external_command()?;
        let activity_mutation = self.owned.begin_managed_activity_mutation();
        let durable_state_store = self.owned.durable_state_store.clone();
        let rebuilt = durable_state_store.with_workflow_runtime_transition_lock(|| {
            self.authorize_current_external_command()?;
            let mut sessions = self.owned.session_store.write();
            sessions.rebuild_workflow_code_definition_with_commit(
                &session_id,
                &workflow_ref,
                expected_workflow_revision,
                &definition,
                source,
                |session, _result| {
                    let mut durable_session = session.clone();
                    durable_session
                        .set_agents(self.owned.agent_store.get_session_agents(&session_id));
                    self.owned
                        .project_session_runtime_view(&mut durable_session);
                    // Use the authoritative workflow-runtime transition as the rebuild record.
                    // A separate audit append cannot be atomic with rollback of this mutation.
                    durable_state_store.persist_workflow_runtime_transition(
                        &durable_session,
                        "workflow_code_source_rebuilt",
                    )?;
                    Ok(())
                },
            )
        });
        let (result, mut session) = match rebuilt {
            Ok(rebuilt) => rebuilt,
            Err(error) => return Err(error),
        };
        session.set_agents(self.owned.agent_store.get_session_agents(&session_id));
        self.owned.project_session_runtime_view(&mut session);
        activity_mutation.record();

        Ok(LocalDaemonResponse::WorkflowCodeSourceRebuilt {
            preview,
            result,
            session,
        })
    }

    pub(super) async fn execute_workflow_code_source_update_from_workflow_request(
        &self,
        request: crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let actor = workflow_code_artifact_actor(caller_user_id, caller_metaagent_id);
        let (workflow, artifact, session, export, preview) = self.with_authorized_app_side_effect({
            let request = request.clone();
            move |app| {
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
            Ok((workflow, artifact, session, export, preview))
            }
        }).await?;
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
        let compile = self
            .compile_workflow_request_source(&request.session_id, &export.source, export.language)
            .await?;
        self.with_authorized_app_side_effect(move |app| {
            let current = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let artifact_name = artifact.metadata.name.clone();
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            if current != workflow
                || registry
                    .get(&artifact_name)?
                    .map(|value| value.metadata)
                    .as_ref()
                    != Some(&artifact.metadata)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.update_from_workflow",
                    message: "workflow or stored source changed during compilation; retry".into(),
                });
            }
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
            self.authorize_current_external_command()?;
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
                crate::session::WorkflowCodeSourceDescriptor {
                    artifact_name,
                    language: artifact.metadata.language,
                    source_sha256: artifact.metadata.source_sha256,
                    origin: crate::session::WorkflowCodeSourceOrigin::Generated,
                },
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
