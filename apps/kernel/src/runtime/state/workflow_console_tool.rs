//! Workflow console runtime-tool handlers.
//!
//! Owns console read/write/clear payloads and source-agent attribution for workflow node runs.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_console_read_tool_result(
        &self,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
        let console = self
            .session_store
            .read()
            .read_workflow_console(&context.session_id, workflow_run.workflow_id())?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "workflow_id": console.workflow_id(),
                "entries": console.entries().iter().map(|entry| serde_json::json!({
                    "timestamp_ms": entry.timestamp_ms(),
                    "source_node_run_id": entry.source_node_run_id(),
                    "source_agent_id": entry.source_agent_id(),
                    "text": entry.text(),
                })).collect::<Vec<_>>(),
            }),
        })
    }

    pub(super) fn workflow_console_write_tool_result(
        &self,
        arguments: &serde_json::Value,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::WorkflowConsoleWriteArgs,
        >(arguments.clone())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_workflow_console_write",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
        let source_agent_id = self.workflow_console_node_agent_id(
            &context.session_id,
            &context.workflow_run_ref,
            &context.workflow_node_run_id,
        );
        let entry = self.session_store.write().append_workflow_console_entry(
            &context.session_id,
            workflow_run.workflow_id(),
            Some(context.workflow_node_run_id.clone()),
            source_agent_id,
            &args.text,
        )?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "timestamp_ms": entry.timestamp_ms(),
                "source_node_run_id": entry.source_node_run_id(),
                "source_agent_id": entry.source_agent_id(),
                "text": entry.text(),
            }),
        })
    }

    pub(super) fn workflow_console_clear_tool_result(
        &self,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
        let console = self
            .session_store
            .write()
            .clear_workflow_console(&context.session_id, workflow_run.workflow_id())?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "cleared": true,
                "workflow_id": console.workflow_id(),
            }),
        })
    }

    fn workflow_console_node_agent_id(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        workflow_node_run_id: &str,
    ) -> Option<String> {
        self.session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)
            .ok()
            .and_then(|workflow_run| {
                workflow_run
                    .node_runs()
                    .iter()
                    .find(|node_run| node_run.id() == workflow_node_run_id)
                    .map(|node_run| node_run.agent_id().to_string())
            })
    }
}
