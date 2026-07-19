use super::*;

impl<'a> KernelSessionService<'a> {
    pub(super) fn apply_workflow_code_definition_with_rebindings_and_alias_base(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
        alias_base: Option<&str>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        let validation = definition.validate_with_limits(limits);
        if !validation.ok {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: format!(
                    "workflow-code definition is invalid: {}",
                    validation
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        let mut definition = definition.clone();
        crate::workflow_code::apply_workflow_code_agent_rebindings(
            &mut definition,
            agent_rebindings,
        )?;
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        let mut target_validation = WorkflowCodeValidationReport {
            ok: true,
            diagnostics: Vec::new(),
        };
        self.append_workflow_code_target_validation(
            session_id,
            &definition,
            &mut target_validation,
            controlled_by_metaagent_id.as_deref(),
        )?;
        if !target_validation.ok {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: workflow_code_validation_error_message(
                    "workflow-code target validation failed",
                    &target_validation,
                ),
            });
        }

        let mut node_agent_ids = BTreeMap::new();
        for node in &definition.nodes {
            let agent_id = match &node.agent {
                WorkflowCodeAgentBinding::Create(agent) => {
                    let mut request = CreateAgentRequest::new(session_id, agent.provider.clone())
                        .with_owner_user_id(created_by_user_id.clone());
                    if let Some(alias) = agent.alias.as_deref() {
                        request = request.with_alias(alias.to_string());
                    }
                    if let Some(model) = agent.model.as_deref() {
                        request = request.with_model(model.to_string());
                    }
                    if let Some(effort) = agent.effort.as_deref() {
                        request = request.with_effort(effort.to_string());
                    }
                    if let Some(account_profile) = agent.account_profile.as_deref() {
                        request = request.with_account_profile(account_profile.to_string());
                    }
                    if let Some(metaagent_id) = controlled_by_metaagent_id.as_deref() {
                        request = request.with_controlled_by_metaagent_id(metaagent_id.to_string());
                    }
                    let created =
                        self.spawn_workflow_code_generated_agent(request, agent.alias.as_deref())?;
                    self.grant_workflow_code_node_extensions(created.id(), &node.extensions)?;
                    created.id().to_string()
                }
                WorkflowCodeAgentBinding::Existing(existing) => {
                    let agent = self.app.agents.get_agent(&existing.agent_ref)?;
                    if agent.session_id() != session_id {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: format!(
                                "existing agent `{}` belongs to session `{}` instead of `{session_id}`",
                                existing.agent_ref,
                                agent.session_id()
                            ),
                        });
                    }
                    if agent.is_metaagent() {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: format!(
                                "invalid_existing_agent_binding: existing agent `{}` is a metaagent and cannot be bound to workflow node `{}`",
                                existing.agent_ref, node.handle
                            ),
                        });
                    }
                    if let Some(metaagent_id) = controlled_by_metaagent_id.as_deref() {
                        if agent.controlled_by_metaagent_id() != Some(metaagent_id) {
                            return Err(DaemonError::LocalTransport {
                                operation: "workflow_code.apply",
                                message: format!(
                                    "metaagent `{metaagent_id}` is not authorized to bind existing agent `{}`",
                                    existing.agent_ref
                                ),
                            });
                        }
                    }
                    self.grant_workflow_code_node_extensions(agent.id(), &node.extensions)?;
                    agent.id().to_string()
                }
            };
            node_agent_ids.insert(node.handle.clone(), agent_id);
        }

        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let report = if let Some(alias_base) = alias_base {
            sessions.apply_workflow_code_definition_with_alias_base(
                session_id,
                &definition,
                &node_agent_ids,
                limits,
                created_by_user_id.clone(),
                controlled_by_metaagent_id.clone(),
                Some(alias_base),
            )?
        } else {
            sessions.apply_workflow_code_definition(
                session_id,
                &definition,
                &node_agent_ids,
                limits,
                created_by_user_id.clone(),
                controlled_by_metaagent_id.clone(),
            )?
        };
        drop(sessions);

        self.app.durable_state_store().append_event(
            "workflow_code.applied",
            Some(report.workflow_id.clone()),
            serde_json::json!({
                "session_id": session_id,
                "created_by_user_id": created_by_user_id,
                "controlled_by_metaagent_id": controlled_by_metaagent_id,
                "report": &report,
            }),
        )?;
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(report)
    }

    fn validate_workflow_code_extension_requirement(
        &mut self,
        workspace_id: &str,
        node_handle: &str,
        target_agent: Option<&crate::agent::AgentInstance>,
        grant: &ExtensionGrant,
    ) -> Result<(), DaemonError> {
        if grant.source == crate::extension::ExtensionSource::Worker {
            return self.validate_workflow_code_worker_extension_requirement(
                node_handle,
                target_agent,
                grant,
            );
        }
        let result = match &grant.kind {
            ExtensionKind::Mcp => crate::runtime::capability_registry::ensure_mcp_exists(
                Some(workspace_id),
                &grant.name,
            ),
            ExtensionKind::Skill => crate::runtime::capability_registry::ensure_skill_exists(
                Some(workspace_id),
                &grant.name,
            ),
            ExtensionKind::Script => {
                let environment =
                    grant
                        .environment
                        .as_deref()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "workflow_code.apply",
                            message: "script extension requirements must include environment"
                                .to_string(),
                        })?;
                crate::runtime::capability_registry::ensure_script_exists(
                    Some(workspace_id),
                    &grant.name,
                )?;
                crate::runtime::capability_registry::ensure_environment_exists(
                    Some(workspace_id),
                    environment,
                )
            }
            ExtensionKind::Connector => {
                crate::runtime::capability_registry::ensure_connector_exists(&grant.name)?;
                if let Some(credential) = grant.credential.as_deref() {
                    crate::runtime::capability_registry::ensure_credential_exists(credential)?;
                }
                crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref()).map(|_| ())
            }
        };

        result.map_err(|error| DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "node `{node_handle}` extension requirement `{}:{}` cannot be satisfied: {error}",
                grant.kind.as_str(),
                grant.name
            ),
        })
    }

    fn validate_workflow_code_worker_extension_requirement(
        &mut self,
        node_handle: &str,
        target_agent: Option<&crate::agent::AgentInstance>,
        grant: &ExtensionGrant,
    ) -> Result<(), DaemonError> {
        let agent = target_agent.ok_or_else(|| DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "node `{node_handle}` requests worker extension `{}:{}`, but newly created workflow agents have no worker placement",
                grant.kind.as_str(),
                grant.name
            ),
        })?;
        let remote = agent.remote_execution().ok_or_else(|| DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "node `{node_handle}` requests worker extension `{}:{}`, but agent `{}` is not assigned to a worker",
                grant.kind.as_str(),
                grant.name,
                agent.agent_ref()
            ),
        })?;
        let relay_config = self.app.relay_config_for_remote_extension_sync(remote);
        let response = self.app.block_on_relay_future(
            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                &relay_config,
                arroba_relay::protocol::ClientTarget {
                    daemon_id: Some(remote.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                crate::transport::relay_peer::RelayPeerRequest::ListLeasedAgentExtensionCatalog {
                    leased_agent_id: remote.leased_agent_id.clone(),
                },
            ),
        )?;
        let crate::transport::relay_peer::RelayPeerResponse::LeasedAgentExtensionCatalogListed {
            leased_agent_id,
            worker_kernel_id,
            entries,
        } = response
        else {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: "worker returned an unexpected extension catalog response".to_string(),
            });
        };
        if leased_agent_id != remote.leased_agent_id
            || worker_kernel_id != remote.worker_kernel_id
            || entries.iter().any(|entry| {
                entry.source != crate::extension::ExtensionSource::Worker
                    || entry.resolved_kernel_id != remote.worker_kernel_id
            })
        {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message:
                    "worker extension catalog response provenance did not match the active lease"
                        .to_string(),
            });
        }
        let entry = entries
            .iter()
            .find(|entry| entry.kind == grant.kind && entry.name == grant.name)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: format!(
                    "node `{node_handle}` worker extension requirement `{}:{}` is unavailable on `{}`",
                    grant.kind.as_str(),
                    grant.name,
                    remote.worker_kernel_id
                ),
            })?;
        validate_workflow_code_worker_catalog_grant(node_handle, grant, entry)
    }

    fn spawn_workflow_code_generated_agent(
        &mut self,
        request: CreateAgentRequest,
        requested_alias: Option<&str>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let Some(alias) = requested_alias else {
            return self.spawn_agent(request);
        };
        let trimmed_alias = alias.trim();
        if trimmed_alias.is_empty() {
            return self.spawn_agent(request);
        }

        for attempt in 0..1000 {
            let candidate_alias = if attempt == 0 {
                trimmed_alias.to_string()
            } else {
                format!("{trimmed_alias}-{}", attempt + 1)
            };
            let candidate = request.clone().with_alias(candidate_alias);
            match self.spawn_agent(candidate) {
                Ok(agent) => return Ok(agent),
                Err(DaemonError::AgentAliasConflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!(
                "could not allocate a unique generated agent alias for `{trimmed_alias}`"
            ),
        })
    }

    fn grant_workflow_code_node_extensions(
        &mut self,
        agent_id: &str,
        grants: &[ExtensionGrant],
    ) -> Result<(), DaemonError> {
        let existing = self.app.agents.get_agent(agent_id)?;
        let mut requested = BTreeMap::new();
        for grant in grants {
            let identity = (grant.kind.clone(), grant.name.clone());
            if let Some(previous_source) = requested.insert(identity, grant.source) {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "extension `{}:{}` appears more than once in the workflow node extension list (sources: {:?}, {:?})",
                        grant.kind.as_str(),
                        grant.name,
                        previous_source,
                        grant.source,
                    ),
                });
            }
            if existing.extension_grants().iter().any(|current| {
                current.source != grant.source
                    && current.kind == grant.kind
                    && current.name == grant.name
            }) {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "extension `{}:{}` is already granted from another source and would collide",
                        grant.kind.as_str(),
                        grant.name
                    ),
                });
            }
        }
        for grant in grants {
            let agent = self.app.agents.grant_extension(agent_id, grant.clone())?;
            self.app.durable_state_store().append_event(
                "agent.extension_granted",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "grant": grant,
                    "source": "workflow_code",
                }),
            )?;
        }
        let agent = self.app.agents.get_agent(agent_id)?;
        if !agent
            .extension_grants_from(crate::extension::ExtensionSource::Worker)
            .is_empty()
        {
            if let Err(error) = self.app.reconcile_remote_worker_extension_grants(&agent) {
                crate::logging::warn_with_fields(
                    "workflow_code.worker_extensions",
                    "stored workflow Worker extension grants for later reconciliation",
                    serde_json::json!({
                        "agent_id": agent.id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn compile_and_apply_workflow_code_javascript(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        self.compile_and_apply_workflow_code_javascript_with_rebindings(
            session_id,
            node_path,
            source,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
            &[],
        )
    }

    pub(crate) fn compile_and_validate_workflow_code_source_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        language: WorkflowCodeLanguage,
        limits: &WorkflowCodeLimitsConfig,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
        caller_metaagent_id: Option<&str>,
    ) -> Result<WorkflowCodeCompileResult, DaemonError> {
        let schema_import_root = self.workflow_code_schema_import_root(session_id)?;
        let mut compile = compile_workflow_code_source_with_schema_import_root(
            node_path,
            source,
            language,
            limits,
            schema_import_root.as_deref(),
        )?;
        let mut definition = compile.definition.clone();
        crate::workflow_code::apply_workflow_code_agent_rebindings(
            &mut definition,
            agent_rebindings,
        )?;
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        if compile.validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &definition,
                &mut compile.validation,
                caller_metaagent_id,
            )?;
            crate::workflow_code::attach_workflow_code_diagnostic_spans(
                &mut compile.validation,
                &compile.source_spans,
            );
        }
        compile.definition = definition;
        Ok(compile)
    }

    pub(crate) fn validate_workflow_code_definition_with_rebindings(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
        caller_metaagent_id: Option<&str>,
    ) -> Result<(WorkflowCodeDefinition, WorkflowCodeValidationReport), DaemonError> {
        let mut definition = definition.clone();
        crate::workflow_code::apply_workflow_code_agent_rebindings(
            &mut definition,
            agent_rebindings,
        )?;
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut definition,
            provider_rebindings,
        )?;
        let mut validation = definition.validate_with_limits(limits);
        if validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &definition,
                &mut validation,
                caller_metaagent_id,
            )?;
        }
        Ok((definition, validation))
    }

    pub(crate) fn compile_and_apply_workflow_code_javascript_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        self.compile_and_apply_workflow_code_source_with_rebindings(
            session_id,
            node_path,
            source,
            WorkflowCodeLanguage::JavaScript,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            provider_rebindings,
            agent_rebindings,
        )
    }

    pub(crate) fn compile_and_apply_workflow_code_source_with_rebindings(
        &mut self,
        session_id: &str,
        node_path: impl AsRef<Path>,
        source: &str,
        language: WorkflowCodeLanguage,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        provider_rebindings: &[crate::workflow_code::WorkflowCodeProviderRebinding],
        agent_rebindings: &[crate::workflow_code::WorkflowCodeAgentRebinding],
    ) -> Result<WorkflowCodeCompileAndApplyResult, DaemonError> {
        let schema_import_root = self.workflow_code_schema_import_root(session_id)?;
        let compile = compile_workflow_code_source_with_schema_import_root(
            node_path,
            source,
            language,
            limits,
            schema_import_root.as_deref(),
        )?;
        let mut rebound_definition = compile.definition.clone();
        crate::workflow_code::apply_workflow_code_agent_rebindings(
            &mut rebound_definition,
            agent_rebindings,
        )?;
        crate::workflow_code::apply_workflow_code_provider_rebindings(
            &mut rebound_definition,
            provider_rebindings,
        )?;
        let mut validation = rebound_definition.validate_with_limits(limits);
        if validation.ok {
            self.append_workflow_code_target_validation(
                session_id,
                &rebound_definition,
                &mut validation,
                controlled_by_metaagent_id.as_deref(),
            )?;
            crate::workflow_code::attach_workflow_code_diagnostic_spans(
                &mut validation,
                &compile.source_spans,
            );
        }
        let apply = self.apply_workflow_code_definition_with_rebindings(
            session_id,
            &rebound_definition,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            &[],
            &[],
        )?;
        Ok(WorkflowCodeCompileAndApplyResult {
            compile: crate::workflow_code::WorkflowCodeCompileResult {
                definition: rebound_definition,
                validation,
                logs: compile.logs,
                source_spans: compile.source_spans,
            },
            apply,
        })
    }

    fn workflow_code_schema_import_root(
        &self,
        session_id: &str,
    ) -> Result<Option<std::path::PathBuf>, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let workspace = std::path::PathBuf::from(session.workspace_id());
        if workspace.is_absolute() {
            Ok(Some(workspace))
        } else {
            Ok(None)
        }
    }

    fn append_workflow_code_target_validation(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        validation: &mut WorkflowCodeValidationReport,
        caller_metaagent_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let session = self.app.sessions().get_session(session_id)?;
        let registry = self.app.providers.registry();
        let generated_agent_count = definition
            .nodes
            .iter()
            .filter(|node| matches!(&node.agent, WorkflowCodeAgentBinding::Create(_)))
            .count();
        let current_agent_count = self.app.agents.get_session_agents(session_id).len();
        if current_agent_count.saturating_add(generated_agent_count) > session.max_agents() as usize
        {
            push_workflow_code_target_validation_error(
                validation,
                "session_agent_limit_exceeded",
                format!(
                    "workflow-code would create {generated_agent_count} agents but session `{session_id}` has {current_agent_count}/{} agents",
                    session.max_agents()
                ),
                None,
            );
        }
        let runtime_queue_limit = self.app.config().max_workflow_queues_per_workflow();
        let materialized_queue_count =
            crate::workflow_code::workflow_code_materialized_queue_count(definition);
        if materialized_queue_count > runtime_queue_limit {
            push_workflow_code_target_validation_error(
                validation,
                "limit_exceeded",
                format!(
                    "queues count {materialized_queue_count} exceeds configured runtime workflow queue limit {runtime_queue_limit}"
                ),
                None,
            );
        }
        if !workflow_code_alias_can_allocate(&session, definition.workflow.alias.as_deref()) {
            let alias = definition
                .workflow
                .alias
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            push_workflow_code_target_validation_error(
                validation,
                "workflow_alias_unavailable",
                format!(
                    "workflow-code alias `{alias}` cannot allocate a unique workflow alias after {} attempts",
                    crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS
                ),
                None,
            );
        }
        for node in &definition.nodes {
            match &node.agent {
                WorkflowCodeAgentBinding::Create(agent) => {
                    let provider = agent.provider.trim();
                    let adapter_key = adapter_key_for_provider(provider);
                    if registry.resolve(adapter_key).is_none() {
                        push_workflow_code_target_validation_error(
                            validation,
                            "unavailable_provider",
                            format!(
                                "node `{}` requests unavailable provider `{provider}`; available providers: {}",
                                node.handle,
                                registry.advertised_provider_ids().join(", ")
                            ),
                            Some(node.handle.clone()),
                        );
                    } else if let Some(model) = agent.model.as_deref() {
                        if let Some(catalog) = self.app.cached_provider_catalog() {
                            let model = model.trim();
                            if model != "default" && !model.is_empty() {
                                if let Some(provider_info) =
                                    catalog.all.iter().find(|item| item.id == provider)
                                {
                                    if !provider_info.models.is_empty()
                                        && !provider_info.models.contains_key(model)
                                    {
                                        push_workflow_code_target_validation_error(
                                            validation,
                                            "unavailable_model",
                                            format!(
                                                "node `{}` requests unavailable model `{model}` for provider `{provider}`; available models: {}",
                                                node.handle,
                                                provider_info
                                                    .models
                                                    .keys()
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ),
                                            Some(node.handle.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                WorkflowCodeAgentBinding::Existing(existing) => {
                    match self.app.agents.get_agent(&existing.agent_ref) {
                        Ok(agent) if agent.session_id() == session_id => {
                            if agent.is_metaagent() {
                                push_workflow_code_target_validation_error(
                                    validation,
                                    "invalid_existing_agent_binding",
                                    format!(
                                        "existing agent `{}` is a metaagent and cannot be bound to workflow node `{}`",
                                        existing.agent_ref, node.handle
                                    ),
                                    Some(node.handle.clone()),
                                );
                            } else if caller_metaagent_id.is_some_and(|metaagent_id| {
                                agent.controlled_by_metaagent_id() != Some(metaagent_id)
                            }) {
                                push_workflow_code_target_validation_error(
                                    validation,
                                    "unauthorized_existing_agent_binding",
                                    format!(
                                        "metaagent is not authorized to bind existing agent `{}`",
                                        existing.agent_ref
                                    ),
                                    Some(node.handle.clone()),
                                );
                            }
                        }
                        Ok(agent) => push_workflow_code_target_validation_error(
                            validation,
                            "invalid_existing_agent_binding",
                            format!(
                                "existing agent `{}` belongs to session `{}` instead of `{session_id}`",
                                existing.agent_ref,
                                agent.session_id()
                            ),
                            Some(node.handle.clone()),
                        ),
                        Err(error) => push_workflow_code_target_validation_error(
                            validation,
                            "invalid_existing_agent_binding",
                            format!(
                                "existing agent `{}` cannot be resolved: {error}",
                                existing.agent_ref
                            ),
                            Some(node.handle.clone()),
                        ),
                    }
                }
            }
            for grant in &node.extensions {
                let target_agent = match &node.agent {
                    WorkflowCodeAgentBinding::Existing(existing) => {
                        self.app.agents.get_agent(&existing.agent_ref).ok()
                    }
                    WorkflowCodeAgentBinding::Create(_) => None,
                };
                if let Err(error) = self.validate_workflow_code_extension_requirement(
                    session.workspace_id(),
                    &node.handle,
                    target_agent.as_ref(),
                    grant,
                ) {
                    push_workflow_code_target_validation_error(
                        validation,
                        "unavailable_extension",
                        error.to_string(),
                        Some(node.handle.clone()),
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn destroy_agent(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote) = agent.remote_execution().cloned() {
            let target = arroba_relay::protocol::ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target.clone(),
                    crate::transport::relay_peer::RelayPeerRequest::DestroyLeasedAgent {
                        leased_agent_id: remote.leased_agent_id.clone(),
                    },
                ),
            )?;
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target,
                    crate::transport::relay_peer::RelayPeerRequest::DestroyExecutionLease {
                        lease_id: remote.execution_lease_id.clone(),
                    },
                ),
            )?;
        }
        let session_id = agent.session_id().to_string();
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let destroyed = self.app.agents.destroy_agent(agent_id, &mut sessions)?;
        drop(sessions);
        self.app
            .external_provider_session_index_store()
            .detach_agent(&session_id, agent_id);
        self.app
            .attached_provider_transcript_cursor_store()
            .detach_agent(&session_id, agent_id);
        self.app.durable_state_store().append_event(
            "agent.deleted",
            Some(destroyed.id().to_string()),
            serde_json::json!({
                "agent": &destroyed,
            }),
        )?;
        Ok(destroyed)
    }

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let (attachment, effect) = self
            .app
            .attachments
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);
        let removed_queued_prompt_count = effect.removed_queued_prompt_count;
        let session_after_detach = SessionStateReader::new(self.app.session_state_store())
            .get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self
                    .app
                    .advance_next_queued_prompt(attachment.session_id(), agent_id)?;
            }
        }

        let remaining_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(attachment.session_id());
        let has_active_prompt = self
            .app
            .prompt_owner_has_any_active_prompt(attachment.session_id())?;
        if remaining_attachment_ids.is_empty() && !has_active_prompt {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                match self.app.providers.get_run(&active_provider_run_id) {
                    Ok(run) if run.state() != ProviderRunState::Ended => {
                        let outcome = self.app.providers.park_run_provider_only(
                            attachment.session_id(),
                            &active_provider_run_id,
                        )?;
                        if SessionStateReader::new(self.app.session_state_store())
                            .get_session(attachment.session_id())?
                            .active_provider_run_id()
                            == Some(outcome.run().id())
                        {
                            SessionStateOwner::new(self.app.session_state_store())
                                .set_active_provider_run(attachment.session_id(), None)?;
                        }
                        self.app.update_provider_run_projection(outcome.into_run());
                    }
                    Ok(_) => {}
                    Err(DaemonError::ProviderRunNotFound { .. }) => {
                        if let Some(mut projected) = self
                            .app
                            .provider_run_projection_store()
                            .get(&active_provider_run_id)
                        {
                            projected.mark_ended();
                            self.app.update_provider_run_projection(projected);
                        }
                        SessionStateOwner::new(self.app.session_state_store())
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            for run in self.app.providers.list_runs() {
                if run.session_id() == attachment.session_id() {
                    crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": effect.removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            self.app.prompt_owner_remove_session(session_id);
            self.app
                .external_provider_session_index_store()
                .detach_session(session_id);
            self.app
                .attached_provider_transcript_cursor_store()
                .detach_session(session_id);
            let ended =
                SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
            self.app.durable_state_store().append_event(
                "session.ended",
                Some(ended.id().to_string()),
                serde_json::json!({
                    "session": &ended,
                    "already_ended": true,
                }),
            )?;
            return Ok(ended);
        }

        let removed_attachments = self.app.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .app
            .providers
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if SessionStateReader::new(self.app.session_state_store())
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                SessionStateOwner::new(self.app.session_state_store())
                    .set_active_provider_run(session_id, None)?;
            }
            let run = outcome.into_run();
            crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(run.id())?;
        }

        let removed_agents = self.app.agents.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.app.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
            }
        }
        self.app.prompt_owner_remove_session(session_id);
        self.app
            .external_provider_session_index_store()
            .detach_session(session_id);
        self.app
            .attached_provider_transcript_cursor_store()
            .detach_session(session_id);
        let mut ended =
            SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
        ended.set_agents(removed_agents);
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        self.app.durable_state_store().append_event(
            "session.ended",
            Some(ended.id().to_string()),
            serde_json::json!({
                "session": &ended,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        )?;
        Ok(ended)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self
            .app
            .agents
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self
            .app
            .should_defer_provider_run_sync_for_focus_change(session_id, agent_id)?
        {
            self.app
                .sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .app
            .sessions()
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.resize_provider_terminal(session_id, &provider_run_id, cols, rows)
    }

    pub(crate) fn resize_provider_terminal(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let _ = crate::app::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }

        if provider_run.endpoint_mode() == AgentEndpointMode::External {
            return Ok(());
        }

        self.app.pty.resize(provider_run_id, cols, rows)
    }
}

fn validate_workflow_code_worker_catalog_grant(
    node_handle: &str,
    grant: &ExtensionGrant,
    entry: &crate::extension::ExtensionCatalogEntry,
) -> Result<(), DaemonError> {
    if let Some(environment) = grant.environment.as_deref() {
        if !entry
            .environments
            .iter()
            .any(|candidate| candidate == environment)
        {
            return Err(workflow_worker_catalog_grant_error(
                node_handle,
                grant,
                format!("environment `{environment}` is unavailable on the worker"),
            ));
        }
    }
    if let Some(credential) = grant.credential.as_deref() {
        if !entry
            .credentials
            .iter()
            .any(|candidate| candidate == credential)
        {
            return Err(workflow_worker_catalog_grant_error(
                node_handle,
                grant,
                format!("credential `{credential}` is unavailable on the worker"),
            ));
        }
    }

    match grant.kind {
        ExtensionKind::Script => {
            if grant.environment.is_none() {
                return Err(workflow_worker_catalog_grant_error(
                    node_handle,
                    grant,
                    "script grants require an explicit worker environment",
                ));
            }
            if grant.max_safety.is_some() {
                return Err(workflow_worker_catalog_grant_error(
                    node_handle,
                    grant,
                    "max_safety is only supported for connector grants",
                ));
            }
        }
        ExtensionKind::Connector => {
            if entry.credential_required && grant.credential.is_none() {
                return Err(workflow_worker_catalog_grant_error(
                    node_handle,
                    grant,
                    "connector requires an explicit worker credential",
                ));
            }
            let max_safety = crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())
                .map_err(|error| {
                    workflow_worker_catalog_grant_error(
                        node_handle,
                        grant,
                        format!("invalid max_safety: {error}"),
                    )
                })?;
            if !entry
                .max_safety
                .iter()
                .any(|candidate| candidate == max_safety.as_str())
            {
                return Err(workflow_worker_catalog_grant_error(
                    node_handle,
                    grant,
                    format!(
                        "max_safety `{}` is unsupported on the worker",
                        max_safety.as_str()
                    ),
                ));
            }
        }
        ExtensionKind::Mcp | ExtensionKind::Skill => {
            if grant.max_safety.is_some() {
                return Err(workflow_worker_catalog_grant_error(
                    node_handle,
                    grant,
                    "max_safety is only supported for connector grants",
                ));
            }
        }
    }

    Ok(())
}

fn workflow_worker_catalog_grant_error(
    node_handle: &str,
    grant: &ExtensionGrant,
    message: impl std::fmt::Display,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workflow_code.apply",
        message: format!(
            "node `{node_handle}` worker extension requirement `{}:{}` cannot be satisfied: {message}",
            grant.kind.as_str(),
            grant.name
        ),
    }
}

#[cfg(test)]
mod extension_grant_tests {
    use super::*;

    fn worker_catalog_entry(kind: ExtensionKind) -> crate::extension::ExtensionCatalogEntry {
        crate::extension::ExtensionCatalogEntry {
            source: crate::extension::ExtensionSource::Worker,
            resolved_kernel_id: "worker-kernel".to_string(),
            kind,
            name: "worker-extension".to_string(),
            description: None,
            definition_hash: None,
            environments: vec!["python".to_string()],
            credentials: vec!["worker-token".to_string()],
            credential_required: false,
            max_safety: vec!["read".to_string(), "write".to_string()],
        }
    }

    #[test]
    fn worker_script_workflow_preflight_requires_supported_environment() {
        let entry = worker_catalog_entry(ExtensionKind::Script);
        let missing_environment = ExtensionGrant::new(ExtensionKind::Script, "worker-extension")
            .from_source(crate::extension::ExtensionSource::Worker);

        let error =
            validate_workflow_code_worker_catalog_grant("planner", &missing_environment, &entry)
                .expect_err("worker scripts must name an environment");
        assert!(error.to_string().contains("explicit worker environment"));

        let unsupported_environment = ExtensionGrant::script("worker-extension", "ruby")
            .from_source(crate::extension::ExtensionSource::Worker);
        let error = validate_workflow_code_worker_catalog_grant(
            "planner",
            &unsupported_environment,
            &entry,
        )
        .expect_err("worker scripts must use a catalog environment");
        assert!(error
            .to_string()
            .contains("environment `ruby` is unavailable"));

        let supported_environment = ExtensionGrant::script("worker-extension", "python")
            .from_source(crate::extension::ExtensionSource::Worker);
        validate_workflow_code_worker_catalog_grant("planner", &supported_environment, &entry)
            .expect("catalog environment should pass worker preflight");
    }

    #[test]
    fn worker_connector_workflow_preflight_requires_local_credential_and_supported_safety() {
        let mut entry = worker_catalog_entry(ExtensionKind::Connector);
        entry.credential_required = true;
        let missing_credential = ExtensionGrant::connector("worker-extension", None, "read")
            .from_source(crate::extension::ExtensionSource::Worker);
        let error =
            validate_workflow_code_worker_catalog_grant("planner", &missing_credential, &entry)
                .expect_err("required worker credential must be explicit");
        assert!(error
            .to_string()
            .contains("requires an explicit worker credential"));

        let unsupported_credential =
            ExtensionGrant::connector("worker-extension", Some("home-token".to_string()), "read")
                .from_source(crate::extension::ExtensionSource::Worker);
        let error =
            validate_workflow_code_worker_catalog_grant("planner", &unsupported_credential, &entry)
                .expect_err("credential must exist in the worker catalog");
        assert!(error
            .to_string()
            .contains("credential `home-token` is unavailable"));

        let invalid_safety =
            ExtensionGrant::connector("worker-extension", Some("worker-token".to_string()), "root")
                .from_source(crate::extension::ExtensionSource::Worker);
        let error = validate_workflow_code_worker_catalog_grant("planner", &invalid_safety, &entry)
            .expect_err("invalid connector safety must fail preflight");
        assert!(error.to_string().contains("invalid max_safety"));

        let unsupported_safety = ExtensionGrant::connector(
            "worker-extension",
            Some("worker-token".to_string()),
            "destructive",
        )
        .from_source(crate::extension::ExtensionSource::Worker);
        let error =
            validate_workflow_code_worker_catalog_grant("planner", &unsupported_safety, &entry)
                .expect_err("catalog must advertise the requested connector safety");
        assert!(error
            .to_string()
            .contains("max_safety `destructive` is unsupported"));

        let supported = ExtensionGrant::connector(
            "worker-extension",
            Some("worker-token".to_string()),
            "write",
        )
        .from_source(crate::extension::ExtensionSource::Worker);
        validate_workflow_code_worker_catalog_grant("planner", &supported, &entry)
            .expect("catalog credential and safety should pass worker preflight");
    }

    #[test]
    fn workflow_node_extension_collision_preflight_is_atomic() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("session should create");
        app.agents_mut()
            .grant_extension(
                agent.id(),
                ExtensionGrant::script("colliding-script", "home-python"),
            )
            .expect("home grant should seed collision");
        let grants = vec![
            ExtensionGrant::new(ExtensionKind::Skill, "safe-worker-skill")
                .from_source(crate::extension::ExtensionSource::Worker),
            ExtensionGrant::script("colliding-script", "worker-python")
                .from_source(crate::extension::ExtensionSource::Worker),
        ];

        let error = KernelSessionService::new(&mut app)
            .grant_workflow_code_node_extensions(agent.id(), &grants)
            .expect_err("cross-source collision should reject the full batch");

        assert!(error.to_string().contains("another source"));
        let unchanged = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should remain present");
        assert!(!unchanged.has_extension_grant_from(
            crate::extension::ExtensionSource::Worker,
            ExtensionKind::Skill,
            "safe-worker-skill",
        ));
        assert!(unchanged.has_extension_grant_from(
            crate::extension::ExtensionSource::Home,
            ExtensionKind::Script,
            "colliding-script",
        ));
        assert_eq!(unchanged.session_id(), session.id());
    }

    #[test]
    fn workflow_worker_grants_remain_desired_state_when_immediate_sync_fails() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (_, agent) = KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("session should create");
        let agent = app
            .agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to a worker");
        let grant = ExtensionGrant::new(ExtensionKind::Skill, "worker-skill")
            .from_source(crate::extension::ExtensionSource::Worker);

        KernelSessionService::new(&mut app)
            .grant_workflow_code_node_extensions(agent.id(), &[grant.clone()])
            .expect("workflow apply should retain desired state when immediate sync fails");

        let stored = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should remain present");
        assert!(stored.has_extension_grant_from(
            crate::extension::ExtensionSource::Worker,
            ExtensionKind::Skill,
            "worker-skill",
        ));
        let sync = stored
            .worker_extension_grant_sync()
            .expect("failed reconciliation should remain visible");
        assert_eq!(
            sync.state,
            crate::extension::RemoteExtensionManifestSyncState::Failed
        );
        assert!(sync.last_error.is_some());
    }
}
