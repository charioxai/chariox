//! Room tools for the Project Environment: an agent reads what this machine has and lacks, runs the
//! saved Setup, and compares the files with the record. Names and states only; never a Secret value.
use super::*;
use crate::local::{LocalDaemonResponse, RunProjectEnvironmentSetupRequest};
use crate::project_environment::{deterministic_proposals, environment_error};
use crate::runtime::kernel_access::error;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const ENVIRONMENT_READ: &str = "chariox.environment.read";
pub(crate) const ENVIRONMENT_SETUP: &str = "chariox.environment.setup";
pub(crate) const ENVIRONMENT_CHECK: &str = "chariox.environment.check";

pub(crate) fn is_environment_tool(name: &str) -> bool {
    matches!(
        name,
        ENVIRONMENT_READ | ENVIRONMENT_SETUP | ENVIRONMENT_CHECK
    )
}

pub(crate) fn environment_tool_specs() -> Vec<RuntimeToolSpec> {
    let empty = serde_json::json!({"type":"object","properties":{},"additionalProperties":false});
    [
        (ENVIRONMENT_READ, "Read this Project's Environment on this machine: Setup state, Variable names, Secrets (name and whether the Vault holds it), Files outside Git, MCPs and Skills the project files declare. Names and states only, never a value."),
        (ENVIRONMENT_SETUP, "Run the Project's saved Setup script in this room's checkout. Returns the operation id at once; read the Environment again for the Setup state when it ends."),
        (ENVIRONMENT_CHECK, "Compare what the Project's files declare (MCPs, Skills, variables named in .env.example) with the saved Environment, with no model. Returns proposals with the file that shows each; nothing is changed."),
    ]
    .into_iter()
    .map(|(name, description)| RuntimeToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema: empty.clone(),
    })
    .collect()
}

impl KernelRuntimeState {
    pub(crate) async fn environment_tool(
        &self,
        token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if arguments
            .as_object()
            .is_none_or(|object| !object.is_empty())
        {
            return Err(error("the Environment tools take no arguments"));
        }
        let context = self.vault_agent_context(token)?;
        let session = self.owned.session_store.get_session(&context.session_id)?;
        let project = self.owned_project(session.project_id(), &context.owner_user_id)?;
        let checkout = self.project_checkout(&project, Some(session.worktree_id()))?;
        let record = self.environment_store().load(project.id())?;
        let payload = match tool_name {
            ENVIRONMENT_READ => {
                let view = self.project_environment_view(&project, checkout, record)?;
                let variables = crate::project_environment::parse_variables(&view.variables)
                    .map_err(environment_error)?
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "exists": view.exists,
                    "revision": view.revision,
                    "machine": view.machine_label,
                    "setup": view.setup_state,
                    "variables": variables,
                    "secrets": view.secrets,
                    "files": view.files,
                    "mcps": view.mcps,
                    "skills": view.skills,
                    "proposals": view.proposals.len(),
                })
            }
            ENVIRONMENT_SETUP => {
                let operation_id = format!("agent-{:032x}", rand::random::<u128>());
                match self.run_project_environment_setup(
                    RunProjectEnvironmentSetupRequest {
                        project_id: project.id().into(),
                        operation_id: operation_id.clone(),
                        checkout_path: Some(checkout.to_string_lossy().into_owned()),
                    },
                    &context.owner_user_id,
                )? {
                    LocalDaemonResponse::EnvironmentOperation { operation } => {
                        serde_json::json!({"operationId": operation.operation_id, "state": operation.state})
                    }
                    _ => return Err(error("Setup did not start")),
                }
            }
            _ => {
                let record = record.ok_or_else(|| {
                    environment_error("this Project has no Environment to check yet")
                })?;
                let proposals = tokio::task::spawn_blocking(move || {
                    deterministic_proposals(&checkout, &record)
                })
                .await
                .map_err(|_| environment_error("reading the Project failed"))?;
                serde_json::json!({ "proposals": proposals })
            }
        };
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
