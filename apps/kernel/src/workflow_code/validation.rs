use super::*;

pub(super) struct WorkflowCodeValidator<'a> {
    limits: &'a WorkflowCodeLimitsConfig,
    diagnostics: Vec<WorkflowCodeValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowCodeCanvasRect {
    kind: &'static str,
    handle: String,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
}

impl WorkflowCodeCanvasRect {
    fn new(
        kind: &'static str,
        handle: String,
        point: WorkflowCodeCanvasPoint,
        width: i64,
        height: i64,
    ) -> Self {
        Self::new_at(kind, handle, point.x as i64, point.y as i64, width, height)
    }

    fn new_at(
        kind: &'static str,
        handle: String,
        left: i64,
        top: i64,
        width: i64,
        height: i64,
    ) -> Self {
        Self {
            kind,
            handle,
            left,
            top,
            width,
            height,
        }
    }

    fn right(&self) -> i64 {
        self.left + self.width
    }

    fn bottom(&self) -> i64 {
        self.top + self.height
    }

    fn conflicts_with(&self, other: &Self, minimum_gap: i64) -> bool {
        !(self.right() + minimum_gap <= other.left
            || other.right() + minimum_gap <= self.left
            || self.bottom() + minimum_gap <= other.top
            || other.bottom() + minimum_gap <= self.top)
    }
}

impl<'a> WorkflowCodeValidator<'a> {
    pub(super) fn new(limits: &'a WorkflowCodeLimitsConfig) -> Self {
        Self {
            limits,
            diagnostics: Vec::new(),
        }
    }

    pub(super) fn validate(&mut self, definition: &WorkflowCodeDefinition) {
        if definition.schema_version != WORKFLOW_CODE_SCHEMA_VERSION {
            self.error(
                "unsupported_schema_version",
                format!(
                    "workflow-code schema_version {} is not supported",
                    definition.schema_version
                ),
                None,
            );
        }

        self.validate_count("nodes", definition.nodes.len(), self.limits.max_nodes);
        self.validate_count("agents", definition.nodes.len(), self.limits.max_agents);
        self.validate_count("edges", definition.edges.len(), self.limits.max_edges);
        self.validate_count(
            "endpoints",
            definition.endpoints.len(),
            self.limits.max_endpoints,
        );
        self.validate_count(
            "queues",
            workflow_code_materialized_queue_count(definition),
            self.limits.max_queues,
        );
        self.validate_count(
            "schedules",
            definition.schedules.len(),
            self.limits.max_watchdogs,
        );
        let generated_prompt_bytes = workflow_code_generated_prompt_bytes(definition);
        if generated_prompt_bytes > self.limits.max_generated_prompt_bytes as usize {
            self.error(
                "limit_exceeded",
                format!(
                    "workflow generated prompt text uses {generated_prompt_bytes} bytes, exceeding configured limit {}",
                    self.limits.max_generated_prompt_bytes
                ),
                None,
            );
        }

        if definition.nodes.is_empty() {
            self.error(
                "missing_node",
                "workflow-code must define at least one node",
                None,
            );
        }
        if definition.endpoints.is_empty() {
            self.error(
                "missing_endpoint",
                "workflow-code must define at least one endpoint",
                None,
            );
        }
        if let Some(max_concurrent) = definition.workflow.max_concurrent {
            if max_concurrent == 0 {
                self.error(
                    "invalid_max_concurrent",
                    "workflow max_concurrent must not be zero",
                    None,
                );
            } else if max_concurrent > self.limits.max_concurrent {
                self.error(
                    "limit_exceeded",
                    format!(
                        "workflow max_concurrent {max_concurrent} exceeds configured limit {}",
                        self.limits.max_concurrent
                    ),
                    None,
                );
            }
        }
        self.validate_alias(definition.workflow.alias.as_deref(), "workflow.alias", None);

        let schema_handles = self.validate_schemas(definition);
        let node_handles = collect_unique_handles(
            self,
            "node",
            definition.nodes.iter().map(|node| node.handle.as_str()),
        );
        let edge_handles = collect_unique_handles(
            self,
            "edge",
            definition.edges.iter().map(|edge| edge.handle.as_str()),
        );
        let endpoint_handles = collect_unique_handles(
            self,
            "endpoint",
            definition
                .endpoints
                .iter()
                .map(|endpoint| endpoint.handle.as_str()),
        );
        let mut queue_handles = collect_unique_handles(
            self,
            "queue",
            definition.queues.iter().map(|queue| queue.handle.as_str()),
        );
        queue_handles.insert("default".to_string());
        self.validate_queues(definition);
        collect_unique_handles(
            self,
            "schedule",
            definition
                .schedules
                .iter()
                .map(|schedule| schedule.handle.as_str()),
        );

        self.validate_schema_ref(
            &schema_handles,
            definition.workflow.run_output_schema.as_deref(),
            "workflow.run_output_schema",
            None,
        );
        let mut existing_agent_refs = BTreeMap::<&str, &str>::new();
        for node in &definition.nodes {
            self.validate_agent_binding(node, &mut existing_agent_refs);
            self.validate_node_extensions(node);
            self.validate_schema_ref(
                &schema_handles,
                node.intermediate_output_schema.as_deref(),
                "node.intermediate_output_schema",
                Some(node.handle.clone()),
            );
            if node.max_turns.is_some_and(|max_turns| max_turns == 0) {
                self.error(
                    "invalid_max_turns",
                    "node max_turns must not be zero",
                    Some(node.handle.clone()),
                );
            }
        }

        for edge in &definition.edges {
            self.validate_ref(
                &node_handles,
                &edge.from_node,
                "edge.from_node",
                Some(edge.handle.clone()),
            );
            self.validate_ref(
                &node_handles,
                &edge.to_node,
                "edge.to_node",
                Some(edge.handle.clone()),
            );
            if edge.from_node == edge.to_node {
                self.error(
                    "invalid_edge",
                    "edge source and target nodes must be different",
                    Some(edge.handle.clone()),
                );
            }
            self.validate_schema_ref(
                &schema_handles,
                edge.handoff_schema.as_deref(),
                "edge.handoff_schema",
                Some(edge.handle.clone()),
            );
        }
        self.validate_edge_pairs(definition);

        for endpoint in &definition.endpoints {
            self.validate_ref(
                &node_handles,
                &endpoint.entry_node,
                "endpoint.entry_node",
                Some(endpoint.handle.clone()),
            );
            if endpoint.max_instances.is_some_and(|max_instances| {
                !(1..=crate::session::MAX_WORKFLOW_ENDPOINT_INSTANCES).contains(&max_instances)
            }) {
                self.error(
                    "invalid_endpoint_max_instances",
                    format!(
                        "endpoint max_instances must be between 1 and {}",
                        crate::session::MAX_WORKFLOW_ENDPOINT_INSTANCES
                    ),
                    Some(endpoint.handle.clone()),
                );
            }
        }
        self.validate_endpoint_aliases(definition);
        self.validate_canvas_layout(definition);
        let reachable_nodes = self.validate_reachable_nodes(definition, &node_handles);
        self.validate_reachable_edges(definition, &node_handles, &reachable_nodes);

        for schedule in &definition.schedules {
            self.validate_ref(
                &endpoint_handles,
                &schedule.endpoint,
                "schedule.endpoint",
                Some(schedule.handle.clone()),
            );
            if let Some(queue) = schedule.queue.as_deref() {
                self.validate_ref(
                    &queue_handles,
                    queue,
                    "schedule.queue",
                    Some(schedule.handle.clone()),
                );
            }
            if let Err(message) = schedule.trigger.validate() {
                self.error(
                    "invalid_schedule_trigger",
                    format!("schedule trigger is invalid: {message}"),
                    Some(schedule.handle.clone()),
                );
            }
            if schedule.invocation_prompt.trim().is_empty() {
                self.error(
                    "invalid_schedule_prompt",
                    "schedule invocation_prompt must not be empty",
                    Some(schedule.handle.clone()),
                );
            }
            if schedule.max_runs.is_some_and(|max_runs| max_runs == 0) {
                self.error(
                    "invalid_schedule_max_runs",
                    "schedule max_runs must not be zero",
                    Some(schedule.handle.clone()),
                );
            }
        }

        let _ = edge_handles;
    }

    fn validate_queues(&mut self, definition: &WorkflowCodeDefinition) {
        let mut aliases = BTreeMap::<String, String>::new();
        for queue in &definition.queues {
            let Some(normalized) = self.validate_alias(
                Some(queue.alias.as_str()),
                "queue.alias",
                Some(queue.handle.clone()),
            ) else {
                continue;
            };
            if queue.handle == "default" && normalized != "default" {
                self.error(
                    "reserved_queue_handle",
                    "queue handle `default` is reserved for the kernel default queue; use alias `default` or choose another handle",
                    Some(queue.handle.clone()),
                );
            }
            if let Some(existing_handle) = aliases.insert(normalized.clone(), queue.handle.clone())
            {
                self.error(
                    "duplicate_queue_alias",
                    format!(
                        "queue alias `{normalized}` is already used by queue `{existing_handle}`"
                    ),
                    Some(queue.handle.clone()),
                );
            }
        }
    }

    fn validate_edge_pairs(&mut self, definition: &WorkflowCodeDefinition) {
        let mut pairs = BTreeMap::<(&str, &str), &str>::new();
        for edge in &definition.edges {
            let pair = (edge.from_node.as_str(), edge.to_node.as_str());
            if let Some(existing_handle) = pairs.insert(pair, edge.handle.as_str()) {
                self.error(
                    "duplicate_edge",
                    format!(
                        "edge `{}` duplicates source-target pair from edge `{existing_handle}`",
                        edge.handle
                    ),
                    Some(edge.handle.clone()),
                );
            }
        }
    }

    fn validate_endpoint_aliases(&mut self, definition: &WorkflowCodeDefinition) {
        let mut aliases = BTreeMap::<String, String>::new();
        for endpoint in &definition.endpoints {
            let Some(normalized) = self.validate_alias(
                endpoint.alias.as_deref(),
                "endpoint.alias",
                Some(endpoint.handle.clone()),
            ) else {
                continue;
            };
            if let Some(existing_handle) =
                aliases.insert(normalized.clone(), endpoint.handle.clone())
            {
                self.error(
                    "duplicate_endpoint_alias",
                    format!(
                        "endpoint alias `{normalized}` is already used by endpoint `{existing_handle}`"
                    ),
                    Some(endpoint.handle.clone()),
                );
            }
        }
    }

    fn validate_reachable_nodes(
        &mut self,
        definition: &WorkflowCodeDefinition,
        node_handles: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        if definition.endpoints.is_empty() {
            return BTreeSet::new();
        }

        let mut reachable = BTreeSet::<String>::new();
        let mut stack = definition
            .endpoints
            .iter()
            .filter_map(|endpoint| {
                node_handles
                    .contains(&endpoint.entry_node)
                    .then(|| endpoint.entry_node.clone())
            })
            .collect::<Vec<_>>();

        while let Some(node_handle) = stack.pop() {
            if !reachable.insert(node_handle.clone()) {
                continue;
            }
            for edge in &definition.edges {
                if edge.from_node == node_handle
                    && node_handles.contains(&edge.to_node)
                    && !reachable.contains(&edge.to_node)
                {
                    stack.push(edge.to_node.clone());
                }
            }
        }

        for node in &definition.nodes {
            if !reachable.contains(&node.handle) {
                self.error(
                    "unreachable_node",
                    "node is not reachable from any workflow endpoint",
                    Some(node.handle.clone()),
                );
            }
        }
        reachable
    }

    fn validate_reachable_edges(
        &mut self,
        definition: &WorkflowCodeDefinition,
        node_handles: &BTreeSet<String>,
        reachable_nodes: &BTreeSet<String>,
    ) {
        for edge in &definition.edges {
            if !node_handles.contains(&edge.from_node) || !node_handles.contains(&edge.to_node) {
                continue;
            }
            if !reachable_nodes.contains(&edge.from_node) {
                self.error(
                    "unreachable_edge",
                    "edge is not reachable from any workflow endpoint",
                    Some(edge.handle.clone()),
                );
            }
        }
    }

    fn validate_canvas_layout(&mut self, definition: &WorkflowCodeDefinition) {
        let mut rects = Vec::<WorkflowCodeCanvasRect>::new();
        for node in &definition.nodes {
            let Some(point) = node.canvas else {
                continue;
            };
            rects.push(WorkflowCodeCanvasRect::new(
                "node",
                node.handle.clone(),
                point,
                WORKFLOW_CODE_CANVAS_NODE_WIDTH,
                WORKFLOW_CODE_CANVAS_NODE_HEIGHT,
            ));
            if node.can_complete_workflow_run == Some(true) {
                rects.push(WorkflowCodeCanvasRect::new_at(
                    "exit_marker",
                    node.handle.clone(),
                    point.x as i64 + WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_X,
                    point.y as i64 + WORKFLOW_CODE_CANVAS_EXIT_MARKER_OFFSET_Y,
                    WORKFLOW_CODE_CANVAS_EXIT_MARKER_WIDTH,
                    WORKFLOW_CODE_CANVAS_EXIT_MARKER_HEIGHT,
                ));
            }
        }
        for endpoint in &definition.endpoints {
            let Some(point) = endpoint.canvas else {
                continue;
            };
            rects.push(WorkflowCodeCanvasRect::new(
                "endpoint",
                endpoint.handle.clone(),
                point,
                WORKFLOW_CODE_CANVAS_ENDPOINT_WIDTH,
                WORKFLOW_CODE_CANVAS_ENDPOINT_HEIGHT,
            ));
        }

        for left_index in 0..rects.len() {
            for right in rects.iter().skip(left_index + 1) {
                let left = &rects[left_index];
                if !left.conflicts_with(right, WORKFLOW_CODE_CANVAS_MIN_GAP) {
                    continue;
                }
                self.error(
                    "canvas_overlap",
                    format!(
                        "{} `{}` conflicts with {} `{}` in {WORKFLOW_CODE_CANVAS_COORDINATE_SPACE}; keep at least {WORKFLOW_CODE_CANVAS_MIN_GAP} canvas units between boxes",
                        left.kind, left.handle, right.kind, right.handle
                    ),
                    Some(right.handle.clone()),
                );
            }
        }
    }

    fn validate_schemas(&mut self, definition: &WorkflowCodeDefinition) -> BTreeSet<String> {
        let handles = collect_unique_handles(
            self,
            "schema",
            definition
                .schemas
                .iter()
                .map(|schema| schema.handle.as_str()),
        );
        let total_schema_bytes = definition
            .schemas
            .iter()
            .map(|schema| {
                serde_json::to_vec(&schema.schema)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0)
            })
            .sum::<usize>();
        if total_schema_bytes > self.limits.max_schema_bytes as usize {
            self.error(
                "limit_exceeded",
                format!(
                    "workflow schemas use {total_schema_bytes} bytes, exceeding configured limit {}",
                    self.limits.max_schema_bytes
                ),
                None,
            );
        }
        for schema in &definition.schemas {
            if let Err(error) = jsonschema::JSONSchema::compile(&schema.schema) {
                self.error(
                    "invalid_schema",
                    format!("schema failed to compile: {error}"),
                    Some(schema.handle.clone()),
                );
            }
        }
        handles
    }

    fn validate_node_extensions(&mut self, node: &WorkflowCodeNodeDefinition) {
        for grant in &node.extensions {
            if grant.name.trim().is_empty() {
                self.error(
                    "invalid_extension_name",
                    "extension name must not be empty",
                    Some(node.handle.clone()),
                );
            }
            match &grant.kind {
                ExtensionKind::Script => {
                    if grant
                        .environment
                        .as_deref()
                        .is_none_or(|environment| environment.trim().is_empty())
                    {
                        self.error(
                            "invalid_extension_environment",
                            "script extension requirements must include environment",
                            Some(node.handle.clone()),
                        );
                    }
                }
                ExtensionKind::Connector => {
                    if let Err(error) =
                        crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())
                    {
                        self.error(
                            "invalid_connector_safety",
                            format!("connector extension safety is invalid: {error}"),
                            Some(node.handle.clone()),
                        );
                    }
                }
                ExtensionKind::Mcp | ExtensionKind::Skill => {}
            }
        }
    }

    fn validate_agent_binding<'b>(
        &mut self,
        node: &'b WorkflowCodeNodeDefinition,
        existing_agent_refs: &mut BTreeMap<&'b str, &'b str>,
    ) {
        match &node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                if agent.provider.trim().is_empty() {
                    self.error(
                        "invalid_agent_provider",
                        "generated agent provider must not be empty",
                        Some(node.handle.clone()),
                    );
                }
            }
            WorkflowCodeAgentBinding::Existing(agent) => {
                if agent.agent_ref.trim().is_empty() {
                    self.error(
                        "invalid_agent_ref",
                        "existing agent_ref must not be empty",
                        Some(node.handle.clone()),
                    );
                    return;
                }
                if let Some(existing_node) =
                    existing_agent_refs.insert(agent.agent_ref.as_str(), node.handle.as_str())
                {
                    self.error(
                        "duplicate_existing_agent",
                        format!(
                            "existing agent_ref `{}` is already bound by node `{existing_node}`",
                            agent.agent_ref
                        ),
                        Some(node.handle.clone()),
                    );
                }
            }
        }
    }

    fn validate_count(&mut self, label: &'static str, actual: usize, limit: u32) {
        if actual > limit as usize {
            self.error(
                "limit_exceeded",
                format!("{label} count {actual} exceeds configured limit {limit}"),
                None,
            );
        }
    }

    fn validate_ref(
        &mut self,
        handles: &BTreeSet<String>,
        value: &str,
        field: &'static str,
        handle: Option<String>,
    ) {
        if value.trim().is_empty() {
            self.error(
                "empty_reference",
                format!("{field} must not be empty"),
                handle,
            );
        } else if !handles.contains(value) {
            self.error(
                "unknown_reference",
                format!("{field} references unknown handle `{value}`"),
                handle,
            );
        }
    }

    fn validate_schema_ref(
        &mut self,
        schema_handles: &BTreeSet<String>,
        value: Option<&str>,
        field: &'static str,
        handle: Option<String>,
    ) {
        if let Some(value) = value {
            self.validate_ref(schema_handles, value, field, handle);
        }
    }

    fn validate_alias(
        &mut self,
        value: Option<&str>,
        field: &'static str,
        handle: Option<String>,
    ) -> Option<String> {
        let value = value?;
        let normalized = value.trim().to_lowercase();
        if normalized.is_empty() {
            self.error(
                "invalid_alias",
                format!("{field} must not be empty"),
                handle,
            );
            return None;
        }
        if !normalized.chars().all(|char| {
            char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
        }) {
            self.error(
                "invalid_alias",
                format!("{field} must use lowercase letters, digits, `-`, or `_`"),
                handle,
            );
            return None;
        }
        Some(normalized)
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, handle: Option<String>) {
        self.diagnostics.push(WorkflowCodeValidationDiagnostic {
            severity: WorkflowCodeValidationSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            handle,
            source_span: None,
        });
    }

    pub(super) fn finish(self) -> WorkflowCodeValidationReport {
        let ok = self
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != WorkflowCodeValidationSeverity::Error);
        WorkflowCodeValidationReport {
            ok,
            diagnostics: self.diagnostics,
        }
    }
}

pub(crate) fn attach_workflow_code_diagnostic_spans(
    validation: &mut WorkflowCodeValidationReport,
    source_spans: &BTreeMap<String, WorkflowCodeSourceSpan>,
) {
    for diagnostic in &mut validation.diagnostics {
        if diagnostic.source_span.is_some() {
            continue;
        }
        let Some(handle) = diagnostic.handle.as_deref() else {
            continue;
        };
        if let Some(source_span) = source_spans.get(handle) {
            diagnostic.source_span = Some(source_span.clone());
        }
    }
}

fn collect_unique_handles<'a>(
    validator: &mut WorkflowCodeValidator<'_>,
    kind: &'static str,
    handles: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for handle in handles {
        if handle.trim().is_empty() {
            validator.error(
                "empty_handle",
                format!("{kind} handle must not be empty"),
                None,
            );
        } else if !seen.insert(handle.to_string()) {
            validator.error(
                "duplicate_handle",
                format!("{kind} handle `{handle}` is duplicated"),
                Some(handle.to_string()),
            );
        }
    }
    seen
}

fn workflow_code_generated_prompt_bytes(definition: &WorkflowCodeDefinition) -> usize {
    fn add_string(total: &mut usize, value: Option<&str>) {
        if let Some(value) = value {
            *total = total.saturating_add(value.len());
        }
    }

    let mut total = 0usize;
    add_string(&mut total, definition.workflow.alias.as_deref());
    add_string(&mut total, definition.workflow.prompt.as_deref());
    for schema in &definition.schemas {
        add_string(&mut total, schema.alias.as_deref());
        add_string(&mut total, schema.description.as_deref());
    }
    for node in &definition.nodes {
        add_string(&mut total, node.public_label.as_deref());
        add_string(&mut total, node.instructions.as_deref());
        add_string(&mut total, node.intermediate_output_schema.as_deref());
        match &node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                add_string(&mut total, agent.alias.as_deref());
                add_string(&mut total, Some(&agent.provider));
                add_string(&mut total, agent.model.as_deref());
                add_string(&mut total, agent.effort.as_deref());
                add_string(&mut total, agent.account_profile.as_deref());
            }
            WorkflowCodeAgentBinding::Existing(agent) => {
                add_string(&mut total, Some(&agent.agent_ref));
            }
        }
    }
    for edge in &definition.edges {
        add_string(&mut total, edge.handoff_schema.as_deref());
    }
    for endpoint in &definition.endpoints {
        add_string(&mut total, endpoint.alias.as_deref());
    }
    for queue in &definition.queues {
        add_string(&mut total, Some(&queue.alias));
    }
    for watchdog in &definition.schedules {
        add_string(&mut total, Some(&watchdog.invocation_prompt));
    }
    total
}

pub(crate) fn workflow_code_materialized_queue_count(definition: &WorkflowCodeDefinition) -> usize {
    1 + definition
        .queues
        .iter()
        .filter(|queue| queue.alias.trim().to_lowercase() != "default")
        .count()
}

pub(super) fn default_workflow_code_schema_version() -> u32 {
    WORKFLOW_CODE_SCHEMA_VERSION
}

pub(super) fn default_enabled() -> bool {
    true
}
