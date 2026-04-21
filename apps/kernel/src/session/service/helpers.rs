use super::*;

pub(super) fn collect_ready_workflow_dispatches(
    next_workflow_node_run_number: &mut u64,
    session_id: &str,
    workflow_id: &str,
    workflow: &WorkflowDefinition,
    workflow_run: &mut WorkflowRun,
) -> Result<Vec<WorkflowDispatch>, DaemonError> {
    let target_node_ids = workflow_run
        .messages()
        .iter()
        .filter(|message| message.consumed_by_node_run_id().is_none())
        .map(|message| message.target_node_id().to_string())
        .collect::<BTreeSet<_>>();
    let mut dispatches = Vec::new();

    for target_node_id in target_node_ids {
        if workflow_run.node_runs().iter().any(|node_run| {
            node_run.node_id() == target_node_id
                && !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                )
        }) {
            continue;
        }

        let target_node = workflow.node(&target_node_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                reference: target_node_id.clone(),
                message: "target node does not exist",
            }
        })?;
        let expected_source_node_ids = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == target_node_id)
            .map(|edge| edge.from_node_id().to_string())
            .collect::<BTreeSet<_>>();
        if expected_source_node_ids.is_empty() {
            continue;
        }

        let source_node_by_run_id = workflow_run
            .node_runs()
            .iter()
            .map(|node_run| (node_run.id().to_string(), node_run.node_id().to_string()))
            .collect::<BTreeMap<_, _>>();

        let mut latest_message_index_by_source = BTreeMap::new();
        for (index, message) in workflow_run.messages().iter().enumerate() {
            if message.target_node_id() != target_node_id
                || message.consumed_by_node_run_id().is_some()
            {
                continue;
            }
            let Some(source_node_run_id) = message.source_node_run_id() else {
                continue;
            };
            let Some(source_node_id) = source_node_by_run_id.get(source_node_run_id) else {
                continue;
            };
            let should_replace = latest_message_index_by_source
                .get(source_node_id.as_str())
                .and_then(|existing_index| workflow_run.messages().get(*existing_index))
                .is_none_or(|existing_message: &WorkflowMessage| {
                    existing_message.created_at_ms() <= message.created_at_ms()
                });
            if should_replace {
                latest_message_index_by_source.insert(source_node_id.to_string(), index);
            }
        }

        if !expected_source_node_ids
            .iter()
            .all(|source_node_id| latest_message_index_by_source.contains_key(source_node_id))
        {
            continue;
        }

        let node_run = WorkflowNodeRun::new(
            next_workflow_node_run_id(next_workflow_node_run_number),
            target_node.id().to_string(),
            target_node.agent_id().to_string(),
            WorkflowNodeRunStatus::Ready,
        );
        let selected_indices = expected_source_node_ids
            .iter()
            .filter_map(|source_node_id| {
                latest_message_index_by_source.get(source_node_id).copied()
            })
            .collect::<Vec<_>>();
        let selected_messages = selected_indices
            .iter()
            .filter_map(|index| workflow_run.messages().get(*index).cloned())
            .collect::<Vec<_>>();
        for (_index, message) in workflow_run.messages_mut().iter_mut().enumerate() {
            if message.target_node_id() != target_node_id
                || message.consumed_by_node_run_id().is_some()
            {
                continue;
            }
            let Some(source_node_run_id) = message.source_node_run_id() else {
                continue;
            };
            let Some(source_node_id) = source_node_by_run_id.get(source_node_run_id) else {
                continue;
            };
            if !expected_source_node_ids.contains(source_node_id) {
                continue;
            }
            message.set_consumed_by_node_run_id(node_run.id().to_string());
        }
        let node_run = workflow_run.add_node_run(node_run);
        dispatches.push(WorkflowDispatch {
            node_run,
            messages: selected_messages,
        });
    }

    Ok(dispatches)
}

fn next_workflow_node_run_id(next_workflow_node_run_number: &mut u64) -> String {
    *next_workflow_node_run_number += 1;
    format!("workflow-node-run-{}", next_workflow_node_run_number)
}

pub(super) fn validate_workflow_edge_output(
    session_id: &str,
    workflow: &WorkflowDefinition,
    edge: &WorkflowEdgeDefinition,
    completion: &Option<WorkflowCompletionSnapshot>,
) -> Result<Option<String>, DaemonError> {
    let Some(schema_ref) = edge.output_schema_ref() else {
        return Ok(None);
    };
    let policy = edge
        .validation_policy()
        .unwrap_or(WorkflowOutputValidationPolicy::Warn);

    let failure = |message: String| -> Result<Option<String>, DaemonError> {
        match policy {
            WorkflowOutputValidationPolicy::Warn => Ok(Some(message)),
            WorkflowOutputValidationPolicy::Halt => {
                Err(DaemonError::WorkflowOutputValidationFailed {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    edge_id: edge.id().to_string(),
                    message,
                })
            }
        }
    };

    let output = completion
        .as_ref()
        .and_then(|value| value.output())
        .ok_or_else(|| "missing workflow output payload".to_string())
        .and_then(|output| {
            serde_json::from_str::<Value>(output.message())
                .map_err(|error| format!("output.message is not valid JSON: {error}"))
        });

    let output_value = match output {
        Ok(value) => value,
        Err(message) => return failure(message),
    };

    let schema_source = std::fs::read_to_string(schema_ref)
        .map_err(|error| format!("schema ref `{schema_ref}` could not be read: {error}"));
    let schema_value = match schema_source {
        Ok(source) => serde_json::from_str::<Value>(&source)
            .map_err(|error| format!("schema ref `{schema_ref}` is not valid JSON: {error}")),
        Err(message) => return failure(message),
    };
    let schema_value = match schema_value {
        Ok(value) => value,
        Err(message) => return failure(message),
    };

    let compiled = JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema_value)
        .map_err(|error| format!("schema ref `{schema_ref}` failed to compile: {error}"));
    let compiled = match compiled {
        Ok(value) => value,
        Err(message) => return failure(message),
    };

    if let Err(errors) = compiled.validate(&output_value) {
        let message = errors
            .into_iter()
            .next()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "schema validation failed".to_string());
        return failure(message);
    }

    Ok(None)
}

pub fn classify_workflow_failure_kind(
    completion: &Option<WorkflowCompletionSnapshot>,
    message: &str,
) -> WorkflowFailureKind {
    if completion.is_none() {
        return WorkflowFailureKind::MissingStructuredOutput;
    }
    if message.contains("missing workflow output payload") {
        return WorkflowFailureKind::MissingStructuredOutput;
    }
    WorkflowFailureKind::OutputValidationFailed
}

pub(super) fn normalize_session_alias(
    alias: Option<String>,
) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias
        .trim()
        .to_lowercase()
        .chars()
        .map(|char| {
            if char.is_ascii_whitespace() {
                '_'
            } else {
                char
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidSessionAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized
        .chars()
        .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || matches!(char, '-' | '_'))
    {
        return Err(DaemonError::InvalidSessionAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

pub(super) fn normalize_workflow_alias(
    alias: Option<String>,
) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidWorkflowAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized.chars().all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    }) {
        return Err(DaemonError::InvalidWorkflowAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

pub(super) fn normalize_workflow_endpoint_alias(
    alias: Option<String>,
) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidWorkflowEndpointAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized.chars().all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    }) {
        return Err(DaemonError::InvalidWorkflowEndpointAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

pub(super) fn normalize_workflow_publication_alias(
    alias: Option<String>,
) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "normalize workflow publication alias",
            message: "alias cannot be empty".to_string(),
        });
    }
    if !normalized.chars().all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    }) {
        return Err(DaemonError::LocalTransport {
            operation: "normalize workflow publication alias",
            message: "alias must use lowercase letters, digits, `-`, or `_`".to_string(),
        });
    }
    Ok(Some(normalized))
}

impl SessionService {
    pub(super) fn next_workflow_id(&mut self) -> String {
        loop {
            self.next_workflow_number = self.next_workflow_number.wrapping_add(1);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(self.next_workflow_number);
            let candidate = format!("{:016x}", nanos ^ self.next_workflow_number.rotate_left(11));
            let exists = self
                .store
                .list()
                .iter()
                .flat_map(|session| session.workflows().iter())
                .any(|workflow| workflow.id() == candidate);
            if !exists {
                return candidate;
            }
        }
    }

    pub(super) fn next_workflow_endpoint_id(&mut self) -> String {
        self.next_workflow_endpoint_number = self.next_workflow_endpoint_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_endpoint_number.rotate_left(9)
        )
    }

    pub(super) fn next_workflow_node_id(&mut self) -> String {
        self.next_workflow_node_number = self.next_workflow_node_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_node_number.rotate_left(7)
        )
    }

    pub(super) fn next_workflow_edge_id(&mut self) -> String {
        self.next_workflow_edge_number = self.next_workflow_edge_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_edge_number.rotate_left(5)
        )
    }

    pub(super) fn next_workflow_run_id(&mut self) -> String {
        self.next_workflow_run_number = self.next_workflow_run_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_run_number.rotate_left(3)
        )
    }

    pub(super) fn next_workflow_node_run_id(&mut self) -> String {
        self.next_workflow_node_run_number = self.next_workflow_node_run_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_node_run_number.rotate_left(13)
        )
    }

    pub(super) fn next_workflow_message_id(&mut self) -> String {
        self.next_workflow_message_number = self.next_workflow_message_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_message_number.rotate_left(15)
        )
    }

    pub(super) fn next_workflow_watchdog_id(&mut self) -> String {
        self.next_workflow_watchdog_number = self.next_workflow_watchdog_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_watchdog_number.rotate_left(1)
        )
    }

    pub(super) fn next_workflow_publication_id(&mut self) -> String {
        self.next_workflow_publication_number =
            self.next_workflow_publication_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_publication_number.rotate_left(19)
        )
    }

    pub(super) fn next_queued_workflow_launch_id(&mut self) -> String {
        self.next_queued_workflow_launch_number =
            self.next_queued_workflow_launch_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_queued_workflow_launch_number.rotate_left(17)
        )
    }
}

pub(super) fn describe_session_match(session: &RuntimeSession) -> String {
    match session.alias() {
        Some(alias) => format!("{} ({alias})", session.id()),
        None => session.id().to_string(),
    }
}
