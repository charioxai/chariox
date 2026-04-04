use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Deserialize;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntryKind;
use crate::provider::LaunchProviderRequest;
use crate::session::{
    WorkflowArtifactRef, WorkflowCompletionSnapshot,
    WorkflowDefinition, WorkflowDispatch, WorkflowEndpointDefinition,
    WorkflowMessage, WorkflowOutputPayload, WorkflowRun,
};

const WORKFLOW_PROMPT_SOURCE_PREFIX: &str = "workflow-run:";
const WORKFLOW_COMPLETION_SUMMARY_LIMIT: usize = 160;
const WORKFLOW_MAX_TURNS_CONFIG_KEY: &str = "workflow.max_turns";

#[derive(Debug, Deserialize)]
struct WorkflowStructuredOutputEnvelope {
    summary: Option<String>,
    output: Option<WorkflowStructuredOutputValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WorkflowStructuredOutputValue {
    Text(String),
    Object { message: String },
}

impl DaemonApp {
    pub(crate) fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        let session = self.sessions().get_session(session_id).ok()?;
        session
            .config_state()
            .values()
            .get(WORKFLOW_MAX_TURNS_CONFIG_KEY)
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
    }

    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.validate_workflow_agents(session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
        )?;
        self.schedule_workflow_run_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, workflow, endpoint))
    }

    fn validate_workflow_agents(
        &self,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let agent_ids = self
            .agents()
            .get_session_agents(session_id)
            .into_iter()
            .map(|agent| agent.id().to_string())
            .collect::<BTreeSet<_>>();
        for node in workflow.nodes() {
            if !agent_ids.contains(node.agent_id()) {
                return Err(DaemonError::WorkflowNodeAgentMissing {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn is_workflow_prompt_source_attachment_id(attachment_id: &str) -> bool {
        attachment_id.starts_with(WORKFLOW_PROMPT_SOURCE_PREFIX)
    }

    pub(crate) fn workflow_prompt_source_attachment_id(workflow_run_id: &str) -> String {
        format!("{WORKFLOW_PROMPT_SOURCE_PREFIX}{workflow_run_id}")
    }

    fn schedule_workflow_run_entry_node(
        &mut self,
        session_id: &str,
        workflow_run: &WorkflowRun,
    ) -> Result<(), DaemonError> {
        let prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        crate::scheduler::SchedulerService::schedule_workflow_node_prompt(
            self,
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.agent_id(),
            node_run.node_id(),
            &self.build_workflow_entry_prompt(
                session_id,
                workflow_run.id(),
                node_run.node_id(),
                prompt,
                self.workflow_node_instruction_reference(
                    session_id,
                    workflow_run.id(),
                    node_run.node_id(),
                ),
                None,
            ),
        )
    }

    pub(crate) fn schedule_workflow_dispatches(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[WorkflowDispatch],
    ) {
        for dispatch in dispatches {
            self.record_notice(
                session_id,
                None,
                self.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            if let Err(error) = crate::scheduler::SchedulerService::schedule_workflow_node_prompt(
                self,
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.agent_id(),
                dispatch.node_run.node_id(),
                &self.build_workflow_entry_prompt(
                    session_id,
                    workflow_run_id,
                    dispatch.node_run.node_id(),
                    "",
                    self.workflow_node_instruction_reference(
                        session_id,
                        workflow_run_id,
                        dispatch.node_run.node_id(),
                    ),
                    Some(&dispatch.messages),
                ),
            ) {
                self.record_notice(
                    session_id,
                    None,
                    self.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                        dispatch.node_run.node_id(),
                        error
                    ),
                );
            }
        }
    }

    fn build_workflow_entry_prompt(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
        prompt: &str,
        instruction_ref: Option<String>,
        handoff_messages: Option<&[WorkflowMessage]>,
    ) -> String {
        let reference_line = instruction_ref
            .as_deref()
            .map(|path| format!("Node instruction reference (daemon-managed): {path}\n\n"))
            .unwrap_or_default();
        let control_line = self
            .workflow_node_control_reference(session_id, workflow_run_id, node_id)
            .map(|path| format!("Control mailbox (daemon-managed): {path}\n\n"))
            .unwrap_or_default();
        let workflow_prompt = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()
            .and_then(|run| run.invocation_prompt().map(str::to_string))
            .unwrap_or_default();
        let handoff_payloads = handoff_messages
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| {
                        serde_json::from_str::<serde_json::Value>(message.handoff_payload())
                            .unwrap_or_else(|_| {
                                serde_json::Value::String(
                                    message.handoff_payload().to_string(),
                                )
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let payloads = serde_json::to_string_pretty(&handoff_payloads)
            .unwrap_or_else(|_| "[]".to_string());
        let payload_block = if handoff_payloads.is_empty() {
            String::new()
        } else {
            format!("Workflow handoff payloads (JSON array):\n{}\n\n", payloads)
        };
        let entry_line = if prompt.trim().is_empty() {
            String::new()
        } else {
            format!("Endpoint prompt:\n{prompt}\n\n")
        };
        format!(
            "{}Workflow-level prompt:\n{}\n\n{}{}{}{}\n",
            entry_line,
            workflow_prompt,
            payload_block,
            reference_line,
            control_line,
            workflow_output_contract_instructions()
        )
    }

    fn workflow_node_instruction_reference(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> Option<String> {
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()?;
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_run.workflow_id())
            .ok()?;
        let node = workflow.node(node_id);
        let attachment_id = Self::workflow_prompt_source_attachment_id(workflow_run_id);
        let root =
            Self::attachment_artifact_root(session_id, &attachment_id, "workflow-instructions");
        let filename = format!("node-{node_id}.md");
        let path = root.join(filename);
        if !path.exists() || node.and_then(|node| node.instructions()).is_some() {
            if let Err(error) = std::fs::create_dir_all(&root) {
                tracing::debug!(
                    ?error,
                    "Failed to create workflow instruction directory at {:?}",
                    root
                );
                return None;
            }
            let content = node
                .and_then(|node| node.instructions())
                .map(|value| value.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
                    )
                });
            if let Err(error) = std::fs::write(&path, content) {
                tracing::debug!(
                    ?error,
                    "Failed to write workflow instruction file at {:?}",
                    path
                );
                return None;
            }
        }
        Some(path.to_string_lossy().to_string())
    }

    fn workflow_node_control_reference(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> Option<String> {
        let attachment_id = Self::workflow_prompt_source_attachment_id(workflow_run_id);
        let root = Self::attachment_artifact_root(session_id, &attachment_id, "workflow-control");
        let path = root.join(format!("node-{node_id}.md"));
        if path.exists() {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        }
    }

    pub(crate) fn ensure_workflow_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        match self.ensure_active_provider_run_for_agent(session_id, agent_id) {
            Ok(provider_run_id) => Ok(provider_run_id),
            Err(DaemonError::NoActiveProviderRun { .. }) => {
                let agent = self.agents().get_agent(agent_id)?;
                let adapter_key = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let provider = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let mut request = LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    provider,
                    "default",
                    agent.model().unwrap_or("default"),
                )
                .with_agent_id(agent.id().to_string())
                .with_variant(agent.effort().map(str::to_string));
                if let Some(worktree_id) = agent.worktree_id() {
                    request = request.with_working_directory(PathBuf::from(worktree_id));
                }
                let provider_run = self.launch_provider(request)?;
                Ok(provider_run.id().to_string())
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn build_workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let session = match self.sessions().get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session while building workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        let Some(workflow_run) = session.workflow_run(workflow_run_id) else {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "workflow run disappeared before completion snapshot could be built",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                }),
            );
            return None;
        };
        let Some(node_run) = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
        else {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "workflow node run disappeared before completion snapshot could be built",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                }),
            );
            return None;
        };
        let history = match self.history.load(&session) {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        let started_at_ms = node_run
            .started_at_ms()
            .unwrap_or_else(|| node_run.created_at_ms());
        let provider_output = history
            .into_iter()
            .filter(|entry| {
                entry.provider_run_id.as_deref() == Some(provider_run_id.as_str())
                    && entry.timestamp_ms >= started_at_ms
                    && entry.kind == SessionHistoryEntryKind::ProviderOutput
            })
            .map(|entry| entry.text)
            .collect::<Vec<_>>()
            .join("");
        let structured_output = parse_workflow_structured_output(&provider_output);
        let summary = structured_output
            .as_ref()
            .and_then(|value| value.summary.as_deref())
            .map(workflow_completion_summary)
            .unwrap_or_else(|| workflow_completion_summary(&provider_output));
        let artifacts =
            self.collect_workflow_artifact_refs(session_id, workflow_run_id, started_at_ms);
        let output_message = structured_output
            .as_ref()
            .and_then(|value| value.output.as_ref())
            .map(|value| match value {
                WorkflowStructuredOutputValue::Text(message) => message.trim().to_string(),
                WorkflowStructuredOutputValue::Object { message } => message.trim().to_string(),
            })
            .filter(|message| !message.is_empty());
        let output = match (output_message, artifacts) {
            (Some(message), artifacts) => Some(WorkflowOutputPayload::new(message, artifacts)),
            (None, artifacts) if !artifacts.is_empty() => {
                Some(WorkflowOutputPayload::new("artifacts attached", artifacts))
            }
            _ => None,
        };
        if summary == "completed" && output.is_none() {
            return None;
        }

        Some(WorkflowCompletionSnapshot::new(summary, output))
    }

    pub(crate) fn write_workflow_control_mailbox(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        warnings: &[crate::session::WorkflowOutputValidationWarning],
    ) {
        let workflow_run = match self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run,
            Err(_) => return,
        };
        let node_id = match workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
        {
            Some(node_run) => node_run.node_id(),
            None => return,
        };
        let attachment_id = Self::workflow_prompt_source_attachment_id(workflow_run_id);
        let root = Self::attachment_artifact_root(session_id, &attachment_id, "workflow-control");
        if let Err(error) = std::fs::create_dir_all(&root) {
            tracing::debug!(
                ?error,
                "Failed to create workflow control directory at {:?}",
                root
            );
            return;
        }
        let path = root.join(format!("node-{node_id}.md"));
        let body = warnings
            .iter()
            .map(|warning| format!("- edge {}: {}", warning.edge_id, warning.message))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!(
            "# Workflow Control Mailbox\n\nValidation warnings for node `{node_id}`:\n{body}\n"
        );
        if let Err(error) = std::fs::write(&path, content) {
            tracing::debug!(
                ?error,
                "Failed to write workflow control mailbox at {:?}",
                path
            );
        }
    }

    fn collect_workflow_artifact_refs(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        started_at_ms: u64,
    ) -> Vec<WorkflowArtifactRef> {
        let attachment_id = Self::workflow_prompt_source_attachment_id(workflow_run_id);
        let mut artifacts = Vec::new();
        for root in Self::attachment_artifact_roots(session_id, &attachment_id) {
            let kind = root
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|value| value.to_str())
                .unwrap_or("artifact")
                .trim_end_matches('s')
                .to_string();
            collect_workflow_artifacts_from_dir(&root, &kind, started_at_ms, &mut artifacts);
        }
        artifacts.sort_by(|left, right| left.id().cmp(right.id()));
        artifacts
    }
}

fn workflow_completion_summary(source: &str) -> String {
    if source.trim().is_empty() {
        return "completed".to_string();
    }
    let normalized = source
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return "completed".to_string();
    }
    if normalized.chars().count() <= WORKFLOW_COMPLETION_SUMMARY_LIMIT {
        return normalized;
    }

    let truncated = normalized
        .chars()
        .take(WORKFLOW_COMPLETION_SUMMARY_LIMIT)
        .collect::<String>();
    format!("{truncated}...")
}

fn workflow_output_contract_instructions() -> &'static str {
    "At the end of this workflow turn, return exactly one fenced ```json block with this shape:\n{\"summary\":\"human-facing summary\",\"output\":{\"message\":\"explicit downstream output message\"}}\nThe summary is for humans and audit. The downstream payload is only output.message plus any workflow-owned artifacts.\n\nIf a handoff payload includes output_schema_ref, validate output.message JSON with ValidateWorkflowOutput before finalizing."
}

fn parse_workflow_structured_output(text: &str) -> Option<WorkflowStructuredOutputEnvelope> {
    let mut cursor = 0usize;
    let mut parsed = None;
    while let Some(start) = text[cursor..].find("```json") {
        let block_start = cursor + start + "```json".len();
        let remaining = &text[block_start..];
        let Some(end) = remaining.find("```") else {
            break;
        };
        let candidate = remaining[..end].trim();
        if let Ok(value) = serde_json::from_str::<WorkflowStructuredOutputEnvelope>(candidate) {
            parsed = Some(value);
        }
        cursor = block_start + end + "```".len();
    }
    parsed
}

fn collect_workflow_artifacts_from_dir(
    root: &std::path::Path,
    kind: &str,
    started_at_ms: u64,
    artifacts: &mut Vec<WorkflowArtifactRef>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_workflow_artifacts_from_dir(&path, kind, started_at_ms, artifacts);
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let modified_at_ms = modified
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        if modified_at_ms < started_at_ms {
            continue;
        }
        let display_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .to_string();
        let path_string = path.to_string_lossy().into_owned();
        artifacts.push(WorkflowArtifactRef::new(
            format!("{kind}:{display_name}"),
            kind.to_string(),
            path_string,
            display_name,
        ));
    }
}
