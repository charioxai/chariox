use super::*;

impl KernelRuntimeState {
    pub(super) async fn meta_workflow_code_create(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeCreateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::CreateWorkflowCodeArtifact(
                    crate::local::CreateWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        language: args
                            .language
                            .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source: args.source,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.created",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_read(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeReadArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::GetWorkflowCodeArtifact(
                    crate::local::GetWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_list(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        _args: MetaWorkflowCodeListArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ListWorkflowCodeArtifacts(
                    crate::local::ListWorkflowCodeArtifactsRequest {
                        session_id: session.id().to_string(),
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_update(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeUpdateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::UpdateWorkflowCodeArtifact(
                    crate::local::UpdateWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        language: args
                            .language
                            .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source: args.source,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.updated",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_delete(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeDeleteArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::DeleteWorkflowCodeArtifact(
                    crate::local::DeleteWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactDeleted { name, path } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.deleted",
                session,
                agent,
                serde_json::json!({ "name": name, "path": path }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodeArtifact(
                    crate::local::ExportWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactExported { package } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.exported",
                session,
                agent,
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_import(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeImportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ImportWorkflowCodeArtifact(
                    crate::local::ImportWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        package: args.package,
                        name: args.name,
                        overwrite: args.overwrite.unwrap_or(false),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactImported { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.imported",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_package_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodePackageExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodePackage(
                    crate::local::ExportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        target: None,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodePackageExported { package } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.package_exported",
                session,
                agent,
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_package_import(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodePackageImportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ImportWorkflowCodePackage(
                    crate::local::ImportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        package: args.package,
                        name: args.name,
                        overwrite: args.overwrite.unwrap_or(false),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodePackageImported { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.package_imported",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_source_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeSourceExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                            name: args.name,
                        },
                        format: args.format,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeSourceExported { export } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.source_exported",
                session,
                agent,
                serde_json::json!({
                    "name": &export.name,
                    "format": export.format,
                    "source_path": &export.source_path,
                    "source_sha256": &export.source_sha256,
                    "source_bytes": export.source_bytes,
                    "definition_sha256": &export.definition_sha256,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_list(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        _args: MetaWorkflowRegistryListArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ListWorkflowRegistry(
                    crate::local::ListWorkflowRegistryRequest {
                        session_id: session.id().to_string(),
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_get(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryGetArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::GetWorkflowRegistryEntry(
                    crate::local::GetWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_add(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryAddArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::AddWorkflowRegistryEntry(
                    crate::local::AddWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        scope: args.scope,
                        source: args.source,
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryAdded { entry } = &response {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.added",
                session,
                agent,
                serde_json::json!({ "entry": entry }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_add_from_workflow(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryAddFromWorkflowArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(
                    crate::local::AddWorkflowRegistryEntryFromWorkflowRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        workflow_ref: args.workflow_ref,
                        scope: args.scope,
                        agent_mode: args.agent_mode,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryAdded { entry } = &response {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.added_from_workflow",
                session,
                agent,
                serde_json::json!({ "entry": entry }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_delete(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryDeleteArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::DeleteWorkflowRegistryEntry(
                    crate::local::DeleteWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        scope: args.scope,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryDeleted { name, path } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.deleted",
                session,
                agent,
                serde_json::json!({ "name": name, "path": path }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_load(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryLoadArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::LoadWorkflowRegistryEntry(
                    crate::local::LoadWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        parameters: args.parameters,
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryLoaded {
            entry,
            result,
            ..
        } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.loaded",
                session,
                agent,
                serde_json::json!({ "entry": entry, "apply": &result.apply }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_registry_run(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryRunArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowRegistryEntry(
                    crate::local::RunWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        parameters: args.parameters,
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryRun {
            entry, result, ..
        } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.run",
                session,
                agent,
                serde_json::json!({
                    "entry": entry,
                    "run": meta_workflow_code_run_audit_payload(result),
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_validate(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeValidateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if let (Some(name), None) = (&args.name, &args.source) {
            let artifact = meta_workflow_code_artifact(session, name)?;
            let provider_rebindings = args.provider_rebindings;
            let agent_rebindings = args.agent_rebindings;
            let session_id = session.id().to_string();
            let metaagent_id = agent.id().to_string();
            let response = self
                .with_app_side_effect(move |app| {
                    let limits = app.config().workflow_code_limits();
                    let (definition, validation) = crate::app::KernelSessionService::new(app)
                        .validate_workflow_code_definition_with_rebindings(
                            &session_id,
                            &artifact.definition,
                            &limits,
                            &provider_rebindings,
                            &agent_rebindings,
                            Some(&metaagent_id),
                        )?;
                    Ok::<_, DaemonError>(crate::local::LocalDaemonResponse::WorkflowCodeValidated {
                        result: crate::workflow_code::WorkflowCodeCompileResult {
                            definition,
                            validation,
                            logs: String::new(),
                            source_spans: std::collections::BTreeMap::new(),
                        },
                    })
                })
                .await?;
            return runtime_tool_result_from_local_response(response);
        }
        let source = meta_workflow_code_source(session, args.name, args.source)?;
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ValidateWorkflowCode(
                    crate::local::ValidateWorkflowCodeRequest {
                        session_id: session.id().to_string(),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source,
                        language: args.language,
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_apply(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeApplyArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let artifact_name = args.name.clone();
        let applies_saved_artifact = matches!((&args.name, &args.source), (Some(_), None));
        let response = if let (Some(name), None) = (&args.name, &args.source) {
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ApplyWorkflowCodeArtifact(
                    crate::local::ApplyWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: name.clone(),
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                    },
                ),
                agent,
            )
            .await?
        } else {
            let source = meta_workflow_code_source(session, args.name, args.source)?;
            self.meta_workflow_code_apply_response(
                session,
                agent,
                source,
                args.node_path,
                args.provider_rebindings,
                args.agent_rebindings,
                args.language,
            )
            .await?
        };
        if let crate::local::LocalDaemonResponse::WorkflowCodeApplied { result, .. } = &response {
            if !applies_saved_artifact {
                self.record_metaagent_workflow_code_artifact_history(
                    session,
                    agent,
                    artifact_name.as_deref(),
                    crate::workflow_code::WorkflowCodeArtifactHistoryAction::Applied,
                    &result.apply,
                );
            }
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.applied",
                session,
                agent,
                serde_json::json!({ "apply": &result.apply }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_run(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeRunArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let artifact_name = args.name.clone();
        let runs_saved_artifact = matches!((&args.name, &args.source), (Some(_), None));
        let response = if let (Some(name), None) = (&args.name, &args.source) {
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowCodeArtifact(
                    crate::local::RunWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: name.clone(),
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?
        } else {
            let source = meta_workflow_code_source(session, args.name, args.source)?;
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowCode(
                    crate::local::RunWorkflowCodeRequest {
                        session_id: session.id().to_string(),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source,
                        language: args.language,
                        provider_rebindings: args.provider_rebindings,
                        agent_rebindings: args.agent_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?
        };
        let run_result = match &response {
            crate::local::LocalDaemonResponse::WorkflowCodeRun { result, .. } => result,
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "meta.workflow_code.run",
                    message: "workflow-code run returned an unexpected response".to_string(),
                });
            }
        };
        self.persist_metaagent_workflow_code_event(
            "metaagent.workflow_code.run",
            session,
            agent,
            meta_workflow_code_run_audit_payload(run_result),
        );
        if !runs_saved_artifact {
            self.record_metaagent_workflow_code_artifact_history(
                session,
                agent,
                artifact_name.as_deref(),
                crate::workflow_code::WorkflowCodeArtifactHistoryAction::Run,
                &run_result.apply.apply,
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    pub(super) async fn meta_workflow_code_apply_response(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        source: String,
        node_path: Option<String>,
        provider_rebindings: Vec<crate::workflow_code::WorkflowCodeProviderRebinding>,
        agent_rebindings: Vec<crate::workflow_code::WorkflowCodeAgentRebinding>,
        language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.meta_execute_workflow_request(
            crate::local::LocalDaemonRequest::ApplyWorkflowCode(
                crate::local::ApplyWorkflowCodeRequest {
                    session_id: session.id().to_string(),
                    node_path: meta_workflow_code_node_path(node_path)?
                        .display()
                        .to_string(),
                    source,
                    language,
                    provider_rebindings,
                    agent_rebindings,
                },
            ),
            agent,
        )
        .await
    }

    async fn meta_execute_workflow_request(
        &self,
        request: crate::local::LocalDaemonRequest,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let (response, _) = self
            .execute_workflow_request(
                request,
                agent.owner_user_id().to_string(),
                Some(agent.id().to_string()),
            )
            .await;
        response
    }

    fn persist_metaagent_workflow_code_event(
        &self,
        kind: &'static str,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        payload: serde_json::Value,
    ) {
        if let Err(error) = self.owned.durable_state_store.append_event(
            kind,
            Some(metaagent.id().to_string()),
            serde_json::json!({
                "session_id": session.id(),
                "metaagent_id": metaagent.id(),
                "owner_user_id": metaagent.owner_user_id(),
                "payload": payload,
                "timestamp_ms": crate::session::unix_epoch_ms(),
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.workflow_code",
                "failed to persist metaagent workflow-code audit",
                serde_json::json!({
                    "kind": kind,
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn record_metaagent_workflow_code_artifact_history(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        artifact_name: Option<&str>,
        action: crate::workflow_code::WorkflowCodeArtifactHistoryAction,
        apply_report: &crate::workflow_code::WorkflowCodeApplyReport,
    ) {
        let Some(artifact_name) = artifact_name else {
            return;
        };
        let actor = crate::workflow_code::WorkflowCodeArtifactActor::new(
            metaagent.owner_user_id().to_string(),
            Some(metaagent.id().to_string()),
        );
        match meta_workflow_code_artifact_registry(session).and_then(|registry| {
            registry.record_apply_history(artifact_name, actor, action, apply_report)
        }) {
            Ok(_) => {}
            Err(error) => crate::logging::warn_with_fields(
                "metaagent.workflow_code",
                "failed to record workflow-code artifact apply history",
                serde_json::json!({
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "artifact": artifact_name,
                    "error": error.to_string(),
                }),
            ),
        }
    }
}

fn meta_workflow_code_node_path(
    node_path: Option<String>,
) -> Result<std::path::PathBuf, DaemonError> {
    node_path
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(crate::workflow_code::discover_workflow_code_node_path)
}

fn meta_workflow_code_source(
    session: &crate::session::RuntimeSession,
    name: Option<String>,
    source: Option<String>,
) -> Result<String, DaemonError> {
    match (name, source) {
        (None, Some(source)) => Ok(source),
        (Some(name), None) => meta_workflow_code_artifact_registry(session)?
            .get(&name)?
            .map(|artifact| artifact.source)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "meta.workflow_code",
                message: format!("workflow-code artifact `{name}` is not saved"),
            }),
        (Some(_), Some(_)) => Err(DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: "pass either name or source, not both".to_string(),
        }),
        (None, None) => Err(DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: "pass either name or source".to_string(),
        }),
    }
}

fn meta_workflow_code_artifact(
    session: &crate::session::RuntimeSession,
    name: &str,
) -> Result<crate::workflow_code::WorkflowCodeArtifact, DaemonError> {
    meta_workflow_code_artifact_registry(session)?
        .get(name)?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: format!("workflow-code artifact `{name}` is not saved"),
        })
}

fn meta_workflow_code_run_audit_payload(
    result: &crate::workflow_code::WorkflowCodeRunResult,
) -> serde_json::Value {
    match &result.invocation {
        crate::workflow_code::WorkflowCodeRunInvocation::Started {
            workflow_run,
            workflow,
            endpoint,
        } => serde_json::json!({
            "outcome": "invoked",
            "apply": &result.apply.apply,
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "workflow_run_id": workflow_run.id(),
        }),
        crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
            queued_prompt,
            workflow,
            endpoint,
        } => serde_json::json!({
            "outcome": "enqueued",
            "apply": &result.apply.apply,
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "queued_prompt_id": queued_prompt.id(),
            "queue_id": queued_prompt.queue_id(),
        }),
    }
}

fn meta_workflow_code_artifact_registry(
    session: &crate::session::RuntimeSession,
) -> Result<crate::workflow_code::WorkflowCodeArtifactRegistry, DaemonError> {
    let mut roots = Vec::new();
    if !session.workspace_id().trim().is_empty() {
        roots.push(
            crate::workflow_code::WorkflowCodeArtifactRegistry::project_root(
                session.workspace_id(),
            ),
        );
    }
    if let Some(root) = crate::workflow_code::WorkflowCodeArtifactRegistry::user_root() {
        roots.push(root);
    }
    Ok(crate::workflow_code::WorkflowCodeArtifactRegistry::new(
        roots,
    ))
}

fn runtime_tool_result_from_local_response(
    response: crate::local::LocalDaemonResponse,
) -> Result<RuntimeToolResult, DaemonError> {
    Ok(RuntimeToolResult {
        ok: true,
        payload: local_response_to_value(&response)?,
    })
}

fn local_response_to_value(
    response: &crate::local::LocalDaemonResponse,
) -> Result<serde_json::Value, DaemonError> {
    serde_json::to_value(response).map_err(|error| DaemonError::LocalTransport {
        operation: "runtime_tool_meta",
        message: format!("failed to serialize workflow-code response: {error}"),
    })
}
