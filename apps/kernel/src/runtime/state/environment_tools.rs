//! Room tools for the Project Environment: an agent reads what this machine has and lacks, brings missing
//! entries from another machine, runs the saved Setup, compares the files with the record, proposes changes
//! and saves the fields. Names and states only; never a Secret value (the Vault capture tools take values).
use super::*;
use crate::local::{
    EnvironmentEntryRef, ImportProjectEnvironmentEntriesRequest, LocalDaemonResponse,
    ProjectEnvironmentExtension, ProjectEnvironmentFilePath, ProjectEnvironmentSecretName,
    ProjectEnvironmentSetupField, RunProjectEnvironmentSetupRequest, SaveProjectEnvironmentRequest,
};
use crate::project_environment::{deterministic_proposals, environment_error, parse_proposals};
use crate::runtime::kernel_access::error;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const ENVIRONMENT_READ: &str = "chariox.environment.read";
pub(crate) const ENVIRONMENT_UPDATE: &str = "chariox.environment.update";
pub(crate) const ENVIRONMENT_SETUP: &str = "chariox.environment.setup";
pub(crate) const ENVIRONMENT_CHECK: &str = "chariox.environment.check";
pub(crate) const ENVIRONMENT_PROPOSE: &str = "chariox.environment.propose";
pub(crate) const ENVIRONMENT_SAVE: &str = "chariox.environment.save";

pub(crate) fn is_environment_tool(name: &str) -> bool {
    matches!(
        name,
        ENVIRONMENT_READ
            | ENVIRONMENT_UPDATE
            | ENVIRONMENT_SETUP
            | ENVIRONMENT_CHECK
            | ENVIRONMENT_PROPOSE
            | ENVIRONMENT_SAVE
    )
}

pub(crate) fn environment_tool_specs() -> Vec<RuntimeToolSpec> {
    let object = |properties: serde_json::Value| serde_json::json!({"type":"object","properties":properties,"additionalProperties":false});
    let entry = serde_json::json!({"type":"object","required":["kind","name"],"properties":{"kind":{"enum":["secret","file","mcp","skill"]},"name":{"type":"string"}},"additionalProperties":false});
    let names = serde_json::json!({"type":"array","items":{"type":"string"}});
    let proposal = serde_json::json!({"type":"object","required":["field","change","text","evidence"],"properties":{
        "field":{"enum":["setup","variables","secret","file"]},"change":{"enum":["add","remove"]},"text":{"type":"string"},
        "evidence":{"type":"object","required":["path","excerpt"],"properties":{"path":{"type":"string"},"excerpt":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false});
    [
        (ENVIRONMENT_READ, "Read this Project's Environment on this machine: Setup state, Variable names, Secrets (name and whether the Vault holds it), Files outside Git, MCPs and Skills (declared by project files, or registered with Chariox), and the user's other machines (`otherMachines`: id and name; this machine is not in the list). Pass operationId to read the state of an operation you started (names and states only, never a value).",
            object(serde_json::json!({"operationId":{"type":"string"}}))),
        (ENVIRONMENT_UPDATE, "Bring the entries this machine lacks (Secrets, Files outside Git, registered MCPs and Skills) from another machine of the user. A machine with no Environment yet (`exists` false in the read result) gets the Environment itself first, then what it lacks. `from` is the id or name of the machine (see `otherMachines` in the read result; optional when there is one other machine), `entries` limits it to some entries (default: everything missing). Returns the operation id at once; read it for the result of each entry.",
            object(serde_json::json!({"from":{"type":"string"},"entries":{"type":"array","items":entry}}))),
        (ENVIRONMENT_SETUP, "Run the Project's saved Setup script in this room's checkout. Returns the operation id at once; read it, or read the Environment again, for the Setup state when it ends.",
            object(serde_json::json!({}))),
        (ENVIRONMENT_CHECK, "Compare what the Project's files declare (MCPs, Skills, variables named in .env.example) with the saved Environment, with no model. Returns proposals with the file that shows each; nothing is changed.",
            object(serde_json::json!({}))),
        (ENVIRONMENT_PROPOSE, "Propose changes to the saved Environment for the owner to accept or dismiss: add or remove a line of Setup or Variables, a Secret name or a File path, each with the project file and line that show why. Nothing changes until the owner accepts a proposal. Returns how many were stored.",
            object(serde_json::json!({"proposals":{"type":"array","items":proposal}}))),
        (ENVIRONMENT_SAVE, "Save the Environment fields as the next revision: Setup script, Variables (KEY=value lines, no values of Secrets), Secret names and File paths. `revision` is the revision you read; a stale one writes nothing. MCPs and Skills stay as they are. Values of Secrets go through the Vault capture tools, never through this tool.",
            object(serde_json::json!({"revision":{"type":"integer"},"setup":{"type":"string"},"variables":{"type":"string"},"secrets":names,"files":names}))),
    ]
    .into_iter()
    .map(|(name, description, input_schema)| RuntimeToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema,
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
        let arguments = arguments
            .as_object()
            .ok_or_else(|| error("the Environment tools take an object"))?;
        let context = self.vault_agent_context(token)?;
        let session = self.owned.session_store.get_session(&context.session_id)?;
        let project = self.tool_project(&session, &context.owner_user_id)?;
        let checkout = self.project_checkout(&project, Some(session.worktree_id()))?;
        let record = self.environment_store().load(project.id())?;
        if matches!(tool_name, ENVIRONMENT_READ | ENVIRONMENT_UPDATE) {
            self.ensure_machine_inventory().await;
        }
        let payload = match tool_name {
            ENVIRONMENT_READ => {
                if let Some(id) = arguments.get("operationId").and_then(|v| v.as_str()) {
                    let operation = self
                        .environment_operation(id)?
                        .filter(|operation| operation.project_id == project.id())
                        .ok_or_else(|| environment_error("this operation is unknown"))?;
                    return Ok(RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "kind": operation.kind,
                            "state": operation.state,
                            "reason": operation.reason,
                            "source": operation.source_label,
                            "entries": operation.entries,
                        }),
                    });
                }
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
                    "otherMachines": self.other_machine_names().into_iter().map(|(id, name)| serde_json::json!({"id": id, "name": name})).collect::<Vec<_>>(),
                    "setup": view.setup_state,
                    "variables": variables,
                    "secrets": view.secrets,
                    "files": view.files,
                    "mcps": view.mcps,
                    "skills": view.skills,
                    "proposals": view.proposals.len(),
                })
            }
            ENVIRONMENT_UPDATE => {
                let entries = arguments
                    .get("entries")
                    .map(|entries| {
                        serde_json::from_value::<Vec<EnvironmentEntryRef>>(entries.clone())
                    })
                    .transpose()
                    .map_err(|_| error("entries are { kind: secret|file|mcp|skill, name }"))?;
                let (source_kernel_id, from) =
                    self.environment_source(arguments.get("from").and_then(|v| v.as_str()))?;
                match self.import_project_environment_entries(
                    ImportProjectEnvironmentEntriesRequest {
                        project_id: project.id().into(),
                        operation_id: format!("agent-{:032x}", rand::random::<u128>()),
                        source_kernel_id,
                        entries,
                        checkout_path: Some(checkout.to_string_lossy().into_owned()),
                    },
                    &context.owner_user_id,
                )? {
                    LocalDaemonResponse::EnvironmentOperation { operation } => {
                        serde_json::json!({"operationId": operation.operation_id, "from": from, "state": operation.state})
                    }
                    _ => return Err(error("the update did not start")),
                }
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
            ENVIRONMENT_CHECK => {
                let record = record.ok_or_else(|| {
                    environment_error("this Project has no Environment to check yet")
                })?;
                let registered = self.project_registrations(&project)?;
                let proposals = tokio::task::spawn_blocking(move || {
                    deterministic_proposals(&checkout, &record, &registered)
                })
                .await
                .map_err(|_| environment_error("reading the Project failed"))?;
                serde_json::json!({ "proposals": proposals })
            }
            ENVIRONMENT_PROPOSE => {
                let record = record.ok_or_else(|| {
                    environment_error("this Project has no Environment to propose changes to")
                })?;
                let proposals = parse_proposals(
                    &serde_json::Value::Object(arguments.clone()).to_string(),
                    &record,
                )
                .map_err(environment_error)?;
                let store = self.proposal_store();
                let mut all = store.list(project.id())?;
                let stored = proposals.len();
                for proposal in proposals {
                    if !all.iter().any(|known| known.id == proposal.id) {
                        all.push(proposal);
                    }
                }
                store.replace(project.id(), all)?;
                serde_json::json!({ "stored": stored })
            }
            _ => {
                let record = record.ok_or_else(|| {
                    environment_error("this Project has no Environment yet; Draft one first")
                })?;
                let text = |key: &str| {
                    arguments
                        .get(key)
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .ok_or_else(|| error(format!("{key} is a string")))
                };
                let list = |key: &str| -> Result<Vec<String>, DaemonError> {
                    arguments
                        .get(key)
                        .and_then(|v| v.as_array())
                        .map(|list| {
                            list.iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .ok_or_else(|| error(format!("{key} is a list of strings")))
                };
                let revision = arguments
                    .get("revision")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| error("revision is the revision you read"))?;
                let extension = |entry: &crate::project_environment::ExtensionEntry| {
                    ProjectEnvironmentExtension {
                        name: entry.name.clone(),
                        source: entry.source.clone(),
                    }
                };
                match self.save_project_environment(
                    SaveProjectEnvironmentRequest {
                        project_id: project.id().into(),
                        expected_revision: revision,
                        setup: ProjectEnvironmentSetupField {
                            script: text("setup")?,
                        },
                        variables: text("variables")?,
                        secrets: list("secrets")?
                            .into_iter()
                            .map(|name| ProjectEnvironmentSecretName { name })
                            .collect(),
                        files: list("files")?
                            .into_iter()
                            .map(|path| ProjectEnvironmentFilePath { path })
                            .collect(),
                        mcps: record.mcps.iter().map(extension).collect(),
                        skills: record.skills.iter().map(extension).collect(),
                    },
                    &context.owner_user_id,
                )? {
                    LocalDaemonResponse::ProjectEnvironmentSaved { environment } => {
                        serde_json::json!({"saved": true, "revision": environment.revision})
                    }
                    LocalDaemonResponse::ProjectEnvironmentConflict { environment } => {
                        serde_json::json!({"saved": false, "reason": "the Environment changed since you read it", "revision": environment.revision})
                    }
                    _ => return Err(error("the Environment was not saved")),
                }
            }
        };
        Ok(RuntimeToolResult { ok: true, payload })
    }

    /// A kernel that has just started has not read the user's machines yet: ask Cloud once when none is cached.
    async fn ensure_machine_inventory(&self) {
        if !self.other_machine_names().is_empty() {
            return;
        }
        let refresh = crate::transport::relay_client::refresh_remote_inventory_projection(
            self.owned.config_projection.clone(),
            self.owned.notification_inventory_projection.clone(),
        );
        match tokio::time::timeout(std::time::Duration::from_secs(5), refresh).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "the machines of the user could not be read for an Environment tool")
            }
            Err(_) => {
                tracing::warn!("reading the machines of the user for an Environment tool timed out")
            }
        }
    }

    /// The other machines of this user that are online: id and the name the user knows them by.
    fn other_machine_names(&self) -> Vec<(String, String)> {
        let own = self.owned.config_projection.snapshot().daemon_id;
        let (_, kernels) = self.owned.notification_inventory_projection.snapshot();
        kernels
            .into_iter()
            .filter(|kernel| kernel.kernel_id != own)
            .map(|kernel| {
                let name = kernel
                    .kernel_alias
                    .or(kernel.machine_alias)
                    .unwrap_or_else(|| kernel.kernel_id.clone());
                (kernel.kernel_id, name)
            })
            .collect()
    }

    /// The machine an update comes from: the one named, or the only other one.
    pub(crate) fn environment_source(
        &self,
        from: Option<&str>,
    ) -> Result<(String, String), DaemonError> {
        let machines = self.other_machine_names();
        let found = match from {
            Some(from) => machines
                .iter()
                .find(|(id, name)| id == from || name.eq_ignore_ascii_case(from)),
            None if machines.len() == 1 => machines.first(),
            None => None,
        };
        found.cloned().ok_or_else(|| {
            let names: Vec<&str> = machines.iter().map(|(_, name)| name.as_str()).collect();
            if names.is_empty() {
                environment_error("no other machine of this user is online")
            } else {
                environment_error(format!(
                    "name the machine to update from: {}",
                    names.join(", ")
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_tools_are_advertised_with_an_object_schema_each() {
        let specs = environment_tool_specs();
        let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "chariox.environment.read",
                "chariox.environment.update",
                "chariox.environment.setup",
                "chariox.environment.check",
                "chariox.environment.propose",
                "chariox.environment.save",
            ]
        );
        assert!(names.iter().all(|name| is_environment_tool(name)));
        assert!(specs
            .iter()
            .all(|spec| spec.input_schema["type"] == "object"));
        assert!(!is_environment_tool("chariox.vault.generate"));
    }
}
