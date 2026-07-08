use super::*;

pub fn export_workflow_code_source_from_session_workflow(
    session: &RuntimeSession,
    workflow_ref: &str,
    format: WorkflowCodeSourceExportFormat,
    agent_mode: WorkflowCodeSourceExportAgentMode,
) -> Result<WorkflowCodeSourceExport, crate::DaemonError> {
    let workflow = session
        .workflows()
        .iter()
        .find(|workflow| {
            workflow.id() == workflow_ref
                || workflow.alias().is_some_and(|alias| alias == workflow_ref)
        })
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.source_export",
            message: format!(
                "workflow `{workflow_ref}` is not present in session `{}`",
                session.id()
            ),
        })?;
    let definition = workflow_code_definition_from_session_workflow(session, workflow, agent_mode)?;
    let name = workflow
        .alias()
        .filter(|alias| !alias.trim().is_empty())
        .unwrap_or(workflow.id());
    export_workflow_code_source_from_definition(name, &definition, format)
}

pub fn export_workflow_code_package_from_session_workflow(
    session: &RuntimeSession,
    workflow_ref: &str,
    package_name: &str,
    agent_mode: WorkflowCodeSourceExportAgentMode,
) -> Result<WorkflowCodeArtifactPackage, crate::DaemonError> {
    validate_registry_name(package_name, "workflow-code package name")?;
    let workflow = session
        .workflows()
        .iter()
        .find(|workflow| {
            workflow.id() == workflow_ref
                || workflow.alias().is_some_and(|alias| alias == workflow_ref)
        })
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.package_export",
            message: format!(
                "workflow `{workflow_ref}` is not present in session `{}`",
                session.id()
            ),
        })?;
    let definition = workflow_code_definition_from_session_workflow(session, workflow, agent_mode)?;
    let source_export = export_workflow_code_source_from_definition(
        package_name,
        &definition,
        WorkflowCodeSourceExportFormat::Inline,
    )?;
    let validation =
        definition.validate_with_limits(&crate::config::WorkflowCodeLimitsConfig::default());
    Ok(WorkflowCodeArtifactPackage {
        package_version: WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: package_name.to_string(),
        language: source_export.language,
        source: source_export.source,
        source_sha256: source_export.source_sha256,
        source_bytes: source_export.source_bytes,
        definition_sha256: source_export.definition_sha256,
        definition,
        validation,
        exported_at_ms: crate::session::unix_epoch_ms(),
    })
}

pub(super) fn export_workflow_code_source_directory(
    name: &str,
    definition: &WorkflowCodeDefinition,
    definition_sha256: String,
) -> Result<WorkflowCodeSourceExport, crate::DaemonError> {
    let mut schema_paths = BTreeMap::new();
    let mut files = Vec::new();
    for schema in &definition.schemas {
        let path = unique_schema_export_path(&schema_paths, schema);
        let contents = serde_json::to_string_pretty(&schema.schema).map_err(|error| {
            crate::DaemonError::LocalTransport {
                operation: "workflow_code.source_export",
                message: format!("failed to serialize workflow-code schema: {error}"),
            }
        })?;
        let contents = format!("{contents}\n");
        schema_paths.insert(schema.handle.clone(), path.clone());
        files.push(WorkflowCodeSourceExportFile {
            path,
            sha256: sha256_hex(contents.as_bytes()),
            contents,
        });
    }

    let source = workflow_code_definition_to_javascript(definition, Some(&schema_paths))?;
    let source_sha256 = sha256_hex(source.as_bytes());
    let source_path = "workflow.js".to_string();
    let manifest = WorkflowCodeSourceExportManifest {
        manifest_version: WORKFLOW_CODE_SOURCE_EXPORT_MANIFEST_VERSION,
        name: name.to_string(),
        language: WorkflowCodeLanguage::JavaScript,
        source_path: source_path.clone(),
        definition_sha256: definition_sha256.clone(),
        source_sha256: source_sha256.clone(),
        schema_paths,
    };
    let manifest_contents = serde_json::to_string_pretty(&manifest).map_err(|error| {
        crate::DaemonError::LocalTransport {
            operation: "workflow_code.source_export",
            message: format!("failed to serialize workflow-code source manifest: {error}"),
        }
    })?;
    let manifest_contents = format!("{manifest_contents}\n");
    files.insert(
        0,
        WorkflowCodeSourceExportFile {
            path: source_path.clone(),
            sha256: source_sha256.clone(),
            contents: source.clone(),
        },
    );
    files.push(WorkflowCodeSourceExportFile {
        path: "manifest.json".to_string(),
        sha256: sha256_hex(manifest_contents.as_bytes()),
        contents: manifest_contents,
    });
    Ok(WorkflowCodeSourceExport {
        name: name.to_string(),
        language: WorkflowCodeLanguage::JavaScript,
        format: WorkflowCodeSourceExportFormat::Directory,
        source_path,
        source_sha256,
        source_bytes: source.len() as u64,
        definition_sha256,
        source,
        files,
    })
}

pub(super) fn export_workflow_code_source_from_definition(
    name: &str,
    definition: &WorkflowCodeDefinition,
    format: WorkflowCodeSourceExportFormat,
) -> Result<WorkflowCodeSourceExport, crate::DaemonError> {
    let definition_sha256 = workflow_code_definition_sha256_hex(definition);
    match format {
        WorkflowCodeSourceExportFormat::Inline => {
            let source = workflow_code_definition_to_javascript(definition, None)?;
            let source_sha256 = sha256_hex(source.as_bytes());
            Ok(WorkflowCodeSourceExport {
                name: name.to_string(),
                language: WorkflowCodeLanguage::JavaScript,
                format,
                source_path: "workflow.js".to_string(),
                source_sha256,
                source_bytes: source.len() as u64,
                definition_sha256,
                source,
                files: Vec::new(),
            })
        }
        WorkflowCodeSourceExportFormat::Directory => {
            export_workflow_code_source_directory(name, definition, definition_sha256)
        }
    }
}

pub(super) fn workflow_code_definition_from_session_workflow(
    session: &RuntimeSession,
    workflow: &crate::session::WorkflowDefinition,
    agent_mode: WorkflowCodeSourceExportAgentMode,
) -> Result<WorkflowCodeDefinition, crate::DaemonError> {
    let canvas = workflow.canvas_layout();
    let mut node_handles = BTreeMap::new();
    let mut used_node_handles = BTreeSet::new();
    for node in workflow.nodes() {
        let agent_alias = session
            .agents()
            .iter()
            .find(|agent| agent.id() == node.agent_id())
            .and_then(|agent| agent.alias());
        let handle = workflow_code_export_handle(agent_alias, node.id(), &mut used_node_handles);
        node_handles.insert(node.id().to_string(), handle);
    }
    let mut endpoint_handles = BTreeMap::new();
    let mut used_endpoint_handles = BTreeSet::new();
    for endpoint in workflow.endpoints() {
        let handle = workflow_code_export_handle(
            endpoint.alias(),
            endpoint.id(),
            &mut used_endpoint_handles,
        );
        endpoint_handles.insert(endpoint.id().to_string(), handle);
    }
    let mut queue_handles = BTreeMap::new();
    let mut used_queue_handles = BTreeSet::new();
    for queue in session
        .workflow_prompt_queues()
        .iter()
        .filter(|queue| queue.workflow_id() == workflow.id())
    {
        let handle =
            workflow_code_export_handle(Some(queue.alias()), queue.id(), &mut used_queue_handles);
        queue_handles.insert(queue.id().to_string(), handle);
    }

    let mut nodes = Vec::new();
    for node in workflow.nodes() {
        let agent = session
            .agents()
            .iter()
            .find(|agent| agent.id() == node.agent_id())
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.source_export",
                message: format!(
                    "workflow node `{}` references missing agent `{}`",
                    node.id(),
                    node.agent_id()
                ),
            })?;
        let agent_binding = match agent_mode {
            WorkflowCodeSourceExportAgentMode::PortableGenerated => {
                WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: agent.alias().map(str::to_string),
                    provider: agent.provider().to_string(),
                    model: agent.model().map(str::to_string),
                    effort: agent.effort().map(str::to_string),
                    account_profile: agent.account_profile().map(str::to_string),
                })
            }
            WorkflowCodeSourceExportAgentMode::ExistingAgents => {
                WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
                    agent_ref: agent.id().to_string(),
                })
            }
        };
        nodes.push(WorkflowCodeNodeDefinition {
            handle: node_handles
                .get(node.id())
                .cloned()
                .unwrap_or_else(|| node.id().to_string()),
            agent: agent_binding,
            public_label: Some(node.public_label().to_string()),
            instructions: node.instructions().map(str::to_string),
            can_complete_workflow_run: Some(node.can_complete_workflow_run()),
            can_emit_intermediate_run_output: Some(node.can_emit_intermediate_run_output()),
            wait_for_all_inputs: Some(node.wait_for_all_inputs()),
            intermediate_output_schema: node.intermediate_output_schema_ref().map(str::to_string),
            max_turns: node.max_turns(),
            extensions: agent.extension_grants().to_vec(),
            canvas: canvas
                .and_then(|layout| layout.nodes.get(node.id()))
                .map(workflow_code_canvas_point_from_layout),
        });
    }

    Ok(WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: WorkflowCodeWorkflow {
            alias: workflow.alias().map(str::to_string),
            prompt: None,
            flush_agent_context_before_run: Some(workflow.flush_agent_context_before_run()),
            max_concurrent: Some(workflow.max_concurrent()),
            run_output_schema: workflow.run_output_schema_ref().map(str::to_string),
        },
        schemas: workflow
            .schemas()
            .iter()
            .map(|schema| WorkflowCodeSchemaDefinition {
                handle: schema.id().to_string(),
                alias: schema.alias().map(str::to_string),
                description: schema.description().map(str::to_string),
                schema: schema.schema().clone(),
            })
            .collect(),
        nodes,
        edges: workflow
            .edges()
            .iter()
            .map(|edge| WorkflowCodeEdgeDefinition {
                handle: edge.id().to_string(),
                from_node: node_handles
                    .get(edge.from_node_id())
                    .cloned()
                    .unwrap_or_else(|| edge.from_node_id().to_string()),
                to_node: node_handles
                    .get(edge.to_node_id())
                    .cloned()
                    .unwrap_or_else(|| edge.to_node_id().to_string()),
                source_side: edge.source_side(),
                target_side: edge.target_side(),
                handoff_schema: edge.handoff_schema_ref().map(str::to_string),
                validation_policy: edge.validation_policy(),
                canvas: canvas
                    .and_then(|layout| layout.edges.get(edge.id()))
                    .map(|layout| WorkflowCodeCanvasEdge {
                        points: layout
                            .waypoints
                            .iter()
                            .map(workflow_code_canvas_point_from_layout)
                            .collect(),
                    }),
            })
            .collect(),
        endpoints: workflow
            .endpoints()
            .iter()
            .map(|endpoint| WorkflowCodeEndpointDefinition {
                handle: endpoint_handles
                    .get(endpoint.id())
                    .cloned()
                    .unwrap_or_else(|| endpoint.id().to_string()),
                entry_node: node_handles
                    .get(endpoint.entry_node_id())
                    .cloned()
                    .unwrap_or_else(|| endpoint.entry_node_id().to_string()),
                alias: endpoint.alias().map(str::to_string),
                canvas: canvas
                    .and_then(|layout| layout.endpoints.get(endpoint.id()))
                    .map(workflow_code_canvas_point_from_layout),
            })
            .collect(),
        queues: session
            .workflow_prompt_queues()
            .iter()
            .filter(|queue| queue.workflow_id() == workflow.id())
            .map(|queue| WorkflowCodeQueueDefinition {
                handle: queue_handles
                    .get(queue.id())
                    .cloned()
                    .unwrap_or_else(|| queue.id().to_string()),
                alias: queue.alias().to_string(),
                priority: queue.priority(),
                enabled: queue.enabled(),
            })
            .collect(),
        schedules: session
            .workflow_schedules()
            .iter()
            .filter(|schedule| schedule.workflow_id() == workflow.id())
            .map(|schedule| WorkflowCodeScheduleDefinition {
                handle: schedule.id().to_string(),
                endpoint: endpoint_handles
                    .get(schedule.endpoint_id())
                    .cloned()
                    .unwrap_or_else(|| schedule.endpoint_id().to_string()),
                queue: schedule.queue_id().map(|queue_id| {
                    queue_handles
                        .get(queue_id)
                        .cloned()
                        .unwrap_or_else(|| queue_id.to_string())
                }),
                enabled: Some(schedule.enabled()),
                trigger: schedule.trigger().clone(),
                invocation_prompt: schedule.invocation_prompt().to_string(),
                overlap_policy: schedule.overlap_policy(),
                max_runs: schedule.max_runs(),
            })
            .collect(),
    })
}

fn workflow_code_export_handle(
    preferred: Option<&str>,
    fallback: &str,
    used: &mut BTreeSet<String>,
) -> String {
    if let Some(preferred) = preferred {
        let normalized = preferred.trim().to_lowercase();
        if !normalized.is_empty()
            && normalized.chars().all(|char| {
                char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
            })
            && used.insert(normalized.clone())
        {
            return normalized;
        }
    }
    used.insert(fallback.to_string());
    fallback.to_string()
}

fn workflow_code_canvas_point_from_layout(
    point: &crate::session::WorkflowCanvasPoint,
) -> WorkflowCodeCanvasPoint {
    WorkflowCodeCanvasPoint {
        x: point.x,
        y: point.y,
    }
}

fn unique_schema_export_path(
    existing: &BTreeMap<String, String>,
    schema: &WorkflowCodeSchemaDefinition,
) -> String {
    let base = schema
        .alias
        .as_deref()
        .filter(|alias| !alias.trim().is_empty())
        .unwrap_or(schema.handle.as_str());
    let base = sanitize_export_stem(base);
    let mut index = 1;
    loop {
        let suffix = if index == 1 {
            String::new()
        } else {
            format!("-{index}")
        };
        let candidate = format!("schemas/{base}{suffix}.json");
        if !existing.values().any(|path| path == &candidate) {
            return candidate;
        }
        index += 1;
    }
}

pub(super) fn workflow_code_definition_to_javascript(
    definition: &WorkflowCodeDefinition,
    schema_paths: Option<&BTreeMap<String, String>>,
) -> Result<String, crate::DaemonError> {
    let mut writer = WorkflowCodeJavascriptWriter::default();
    writer.line("// Generated by arroba workflow-code source export.");
    writer.line("async function defineWorkflow(workflow) {");
    writer.indent += 1;
    for schema in &definition.schemas {
        writer.write_schema(schema, schema_paths)?;
    }
    writer.write_workflow_define(&definition.workflow)?;
    for node in &definition.nodes {
        writer.write_node(node)?;
    }
    for edge in &definition.edges {
        writer.write_edge(edge)?;
    }
    for endpoint in &definition.endpoints {
        writer.write_endpoint(endpoint)?;
    }
    for queue in &definition.queues {
        writer.write_queue(queue)?;
    }
    for schedule in &definition.schedules {
        writer.write_schedule(schedule)?;
    }
    writer.indent -= 1;
    writer.line("}");
    Ok(writer.output)
}

#[derive(Default)]
struct WorkflowCodeJavascriptWriter {
    output: String,
    indent: usize,
    vars: BTreeMap<String, String>,
    used_vars: BTreeSet<String>,
}

impl WorkflowCodeJavascriptWriter {
    fn line(&mut self, line: impl AsRef<str>) {
        self.output.push_str(&"  ".repeat(self.indent));
        self.output.push_str(line.as_ref());
        self.output.push('\n');
    }

    fn var_for(&mut self, kind: &str, handle: &str) -> String {
        let key = var_key(kind, handle);
        if let Some(var) = self.vars.get(&key) {
            return var.clone();
        }
        let stem = sanitize_identifier_stem(handle);
        let mut candidate = format!("{kind}_{stem}");
        let mut index = 2;
        while self.used_vars.contains(&candidate) {
            candidate = format!("{kind}_{stem}_{index}");
            index += 1;
        }
        self.used_vars.insert(candidate.clone());
        self.vars.insert(key, candidate.clone());
        candidate
    }

    fn existing_var(&self, handle: &str, kind: &str) -> Result<String, crate::DaemonError> {
        self.vars
            .get(&var_key(kind, handle))
            .cloned()
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.source_export",
                message: format!(
                    "cannot export workflow-code source: unknown {kind} handle `{handle}`"
                ),
            })
    }

    fn write_workflow_define(
        &mut self,
        workflow: &WorkflowCodeWorkflow,
    ) -> Result<(), crate::DaemonError> {
        let mut fields = Vec::new();
        push_json_field(&mut fields, "alias", &workflow.alias)?;
        push_json_field(&mut fields, "prompt", &workflow.prompt)?;
        push_json_field(
            &mut fields,
            "flushAgentContextBeforeRun",
            &workflow.flush_agent_context_before_run,
        )?;
        push_json_field(&mut fields, "maxConcurrent", &workflow.max_concurrent)?;
        push_ref_field(
            &mut fields,
            "runOutputSchema",
            &workflow.run_output_schema,
            "schema",
            &self.vars,
        )?;
        if !fields.is_empty() {
            self.line(format!("workflow.define({{ {} }})", fields.join(", ")));
        }
        Ok(())
    }

    fn write_schema(
        &mut self,
        schema: &WorkflowCodeSchemaDefinition,
        schema_paths: Option<&BTreeMap<String, String>>,
    ) -> Result<(), crate::DaemonError> {
        let var = self.var_for("schema", &schema.handle);
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(schema.handle.clone()))?;
        push_json_field(&mut fields, "alias", &schema.alias)?;
        push_json_field(&mut fields, "description", &schema.description)?;
        match schema_paths.and_then(|paths| paths.get(&schema.handle)) {
            Some(path) => {
                let options = fields.join(", ");
                self.line(format!(
                    "const {var} = workflow.schemaFromFile({}, {{ {options} }})",
                    js_json(path)?
                ));
            }
            None => {
                push_json_field(&mut fields, "schema", &Some(schema.schema.clone()))?;
                self.line(format!(
                    "const {var} = workflow.schema({{ {} }})",
                    fields.join(", ")
                ));
            }
        }
        Ok(())
    }

    fn write_node(&mut self, node: &WorkflowCodeNodeDefinition) -> Result<(), crate::DaemonError> {
        let var = self.var_for("node", &node.handle);
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(node.handle.clone()))?;
        fields.push(format!("agent: {}", agent_binding_js(&node.agent)?));
        push_json_field(&mut fields, "publicLabel", &node.public_label)?;
        push_json_field(&mut fields, "instructions", &node.instructions)?;
        push_json_field(
            &mut fields,
            "canCompleteWorkflowRun",
            &node.can_complete_workflow_run,
        )?;
        push_json_field(
            &mut fields,
            "canEmitIntermediateRunOutput",
            &node.can_emit_intermediate_run_output,
        )?;
        push_json_field(&mut fields, "waitForAllInputs", &node.wait_for_all_inputs)?;
        push_ref_field(
            &mut fields,
            "intermediateOutputSchema",
            &node.intermediate_output_schema,
            "schema",
            &self.vars,
        )?;
        push_json_field(&mut fields, "maxTurns", &node.max_turns)?;
        if !node.extensions.is_empty() {
            fields.push(format!("extensions: {}", js_json(&node.extensions)?));
        }
        push_json_field(&mut fields, "canvas", &node.canvas)?;
        self.line(format!(
            "const {var} = workflow.node({{ {} }})",
            fields.join(", ")
        ));
        Ok(())
    }

    fn write_edge(&mut self, edge: &WorkflowCodeEdgeDefinition) -> Result<(), crate::DaemonError> {
        let var = self.var_for("edge", &edge.handle);
        let from = self.existing_var(&edge.from_node, "node")?;
        let to = self.existing_var(&edge.to_node, "node")?;
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(edge.handle.clone()))?;
        push_json_field(&mut fields, "sourceSide", &edge.source_side)?;
        push_json_field(&mut fields, "targetSide", &edge.target_side)?;
        push_ref_field(
            &mut fields,
            "handoffSchema",
            &edge.handoff_schema,
            "schema",
            &self.vars,
        )?;
        push_json_field(&mut fields, "validationPolicy", &edge.validation_policy)?;
        push_json_field(&mut fields, "canvas", &edge.canvas)?;
        self.line(format!(
            "const {var} = workflow.edge({from}, {to}, {{ {} }})",
            fields.join(", ")
        ));
        Ok(())
    }

    fn write_endpoint(
        &mut self,
        endpoint: &WorkflowCodeEndpointDefinition,
    ) -> Result<(), crate::DaemonError> {
        let var = self.var_for("endpoint", &endpoint.handle);
        let entry = self.existing_var(&endpoint.entry_node, "node")?;
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(endpoint.handle.clone()))?;
        push_json_field(&mut fields, "alias", &endpoint.alias)?;
        push_json_field(&mut fields, "canvas", &endpoint.canvas)?;
        self.line(format!(
            "const {var} = workflow.endpoint({entry}, {{ {} }})",
            fields.join(", ")
        ));
        Ok(())
    }

    fn write_queue(
        &mut self,
        queue: &WorkflowCodeQueueDefinition,
    ) -> Result<(), crate::DaemonError> {
        let var = self.var_for("queue", &queue.handle);
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(queue.handle.clone()))?;
        push_json_field(&mut fields, "alias", &Some(queue.alias.clone()))?;
        push_json_field(&mut fields, "priority", &Some(queue.priority))?;
        push_json_field(&mut fields, "enabled", &Some(queue.enabled))?;
        self.line(format!(
            "const {var} = workflow.queue({{ {} }})",
            fields.join(", ")
        ));
        Ok(())
    }

    fn write_schedule(
        &mut self,
        schedule: &WorkflowCodeScheduleDefinition,
    ) -> Result<(), crate::DaemonError> {
        let var = self.var_for("schedule", &schedule.handle);
        let endpoint = self.existing_var(&schedule.endpoint, "endpoint")?;
        let mut fields = Vec::new();
        push_json_field(&mut fields, "handle", &Some(schedule.handle.clone()))?;
        push_ref_field(&mut fields, "queue", &schedule.queue, "queue", &self.vars)?;
        push_json_field(&mut fields, "enabled", &schedule.enabled)?;
        push_json_field(&mut fields, "trigger", &Some(schedule.trigger.clone()))?;
        push_json_field(
            &mut fields,
            "invocationPrompt",
            &Some(schedule.invocation_prompt.clone()),
        )?;
        push_json_field(&mut fields, "overlapPolicy", &Some(schedule.overlap_policy))?;
        push_json_field(&mut fields, "maxRuns", &schedule.max_runs)?;
        self.line(format!(
            "const {var} = workflow.schedule({endpoint}, {{ {} }})",
            fields.join(", ")
        ));
        Ok(())
    }
}

fn agent_binding_js(agent: &WorkflowCodeAgentBinding) -> Result<String, crate::DaemonError> {
    match agent {
        WorkflowCodeAgentBinding::Create(agent) => {
            let mut fields = Vec::new();
            push_json_field(&mut fields, "alias", &agent.alias)?;
            push_json_field(&mut fields, "provider", &Some(agent.provider.clone()))?;
            push_json_field(&mut fields, "model", &agent.model)?;
            push_json_field(&mut fields, "effort", &agent.effort)?;
            push_json_field(&mut fields, "accountProfile", &agent.account_profile)?;
            Ok(format!("workflow.newAgent({{ {} }})", fields.join(", ")))
        }
        WorkflowCodeAgentBinding::Existing(agent) => Ok(format!(
            "workflow.existingAgent({})",
            js_json(&agent.agent_ref)?
        )),
    }
}

fn push_ref_field(
    fields: &mut Vec<String>,
    name: &str,
    value: &Option<String>,
    kind: &str,
    vars: &BTreeMap<String, String>,
) -> Result<(), crate::DaemonError> {
    let Some(handle) = value else {
        return Ok(());
    };
    let Some(var) = vars.get(&var_key(kind, handle)) else {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_code.source_export",
            message: format!(
                "cannot export workflow-code source: unknown referenced handle `{handle}`"
            ),
        });
    };
    fields.push(format!("{name}: {var}"));
    Ok(())
}

fn var_key(kind: &str, handle: &str) -> String {
    format!("{kind}:{handle}")
}

fn push_json_field<T: Serialize>(
    fields: &mut Vec<String>,
    name: &str,
    value: &Option<T>,
) -> Result<(), crate::DaemonError> {
    if let Some(value) = value {
        fields.push(format!("{name}: {}", js_json(value)?));
    }
    Ok(())
}

fn js_json<T: Serialize>(value: &T) -> Result<String, crate::DaemonError> {
    serde_json::to_string(value).map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_code.source_export",
        message: format!("failed to serialize workflow-code source: {error}"),
    })
}

fn sanitize_identifier_stem(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "item".to_string()
    } else if out.as_bytes()[0].is_ascii_digit() {
        format!("item_{out}")
    } else {
        out.to_string()
    }
}

fn sanitize_export_stem(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "schema".to_string()
    } else {
        out.to_string()
    }
}
