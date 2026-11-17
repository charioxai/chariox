//! MP-08 / MP-10 / MP-11: selected-run native inventory on the execution kernel.
use super::*;
use crate::provider::skill_inventory::{
    NativeSkillDiscoveryRequest, NativeSkillInventory, NativeSkillInventoryCache,
};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct ExecutionInventory {
    version: Option<String>,
    inventory: NativeSkillInventoryCache,
}
#[derive(Default)]
pub(super) struct SkillInventories {
    executions: StdMutex<BTreeMap<String, Arc<StdMutex<ExecutionInventory>>>>,
}
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.native_inventory",
        message: message.into(),
    }
}
impl SkillInventories {
    #[cfg(test)]
    pub(super) fn set_test_version(&self, run_id: &str, version: &str) {
        self.execution(run_id).lock().unwrap().version = Some(version.into());
    }
    fn execution(&self, run_id: &str) -> Arc<StdMutex<ExecutionInventory>> {
        let mut entries = self
            .executions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !entries.contains_key(run_id) && entries.len() >= 128 {
            if let Some(key) = entries.keys().next().cloned() {
                entries.remove(&key);
            }
        }
        entries.entry(run_id.into()).or_default().clone()
    }
}
impl KernelRuntimeState {
    pub(super) fn local_skill_inventory(
        &self,
        agent_id: &str,
        catalog_revision: u64,
        refresh: bool,
    ) -> Result<NativeSkillInventory, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Err(failure(
                "native inventory must be resolved on this agent's leased execution kernel",
            ));
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let run=self.owned.provider_store.get_run_for_agent(session.id(),agent.id())
            .ok_or_else(||failure("native inventory requires the focused provider to start; submit a turn or attach its native TUI"))?;
        self.local_skill_inventory_for_run(&run, catalog_revision, refresh)
    }
    pub(super) fn local_skill_inventory_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        catalog_revision: u64,
        refresh: bool,
    ) -> Result<NativeSkillInventory, DaemonError> {
        let session = self.owned.session_store.get_session(run.session_id())?;
        let agent = run
            .agent_instance_id()
            .and_then(|id| self.owned.agent_store.get_agent(id).ok());
        self.owned.skill_inventories.for_run(
            run,
            &session,
            agent.as_ref(),
            &self.owned.config_projection.snapshot(),
            catalog_revision,
            refresh,
        )
    }
}
impl SkillInventories {
    pub(super) fn for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        session: &crate::session::RuntimeSession,
        agent: Option<&crate::agent::AgentInstance>,
        config: &crate::config::DaemonConfig,
        catalog_revision: u64,
        refresh: bool,
    ) -> Result<NativeSkillInventory, DaemonError> {
        let project_id = session.project_id();
        let environment = run.native_skill_discovery_environment();
        if environment.is_empty() {
            return Err(failure(
                "provider launch discovery settings are unavailable; reattach the provider",
            ));
        }
        let execution = self.execution(run.id());
        let mut execution = execution
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if execution.version.is_none() {
            let version = match crate::provider::canonical_provider_family(run.provider()) {
                Some("codex") => crate::provider::CodexClient::new(
                    run.id(),
                    run.structured_endpoint()
                        .ok_or_else(|| failure("Codex has no official app-server endpoint"))?,
                )?
                .native_skill_server_version()?,
                Some("opencode") => crate::provider::OpenCodeClient::new(
                    run.id(),
                    run.structured_endpoint()
                        .ok_or_else(|| failure("OpenCode has no official server endpoint"))?,
                )?
                .native_skill_server_version()?,
                Some("claude") => crate::provider::probe_claude_skill_version(
                    &crate::provider::resolve_claude_executable()?,
                    environment,
                )?,
                _ => return Err(failure("this provider has no native skills inventory seam")),
            };
            execution.version = Some(version);
        }
        let version = execution.version.clone().expect("resolved above");
        let root = PathBuf::from(
            agent
                .and_then(|agent| agent.worktree_id())
                .unwrap_or_else(|| session.worktree_id()),
        );
        let root = std::fs::canonicalize(root)
            .map_err(|_| failure("provider worktree boundary is unavailable"))?;
        let execution_id = format!("{}:{}", config.daemon_id, run.id());
        crate::provider::skill_inventory::inventory_for_run(
            run,
            &mut execution.inventory,
            NativeSkillDiscoveryRequest {
                provider_version: &version,
                execution_id: &execution_id,
                project_id: Some(project_id),
                catalog_revision,
                source_revision: 0,
                worktree_root: &root,
                configured_skill_roots: &[],
            },
            refresh,
        )
    }
}

/// Public Details projection of one harness-native entry on the execution kernel.
pub(super) fn native_details(
    inventory: &NativeSkillInventory,
    selection: &crate::skill::v2::SkillSelection,
    source_machine: String,
) -> Result<crate::skill::v2::NativeSkillDetails, DaemonError> {
    use crate::skill::v2::{NativeSkillDetails, NativeSkillProvenance};
    let details = inventory.details(selection)?;
    let metadata = details
        .body
        .as_deref()
        .map(crate::skill::frontmatter::parse_frontmatter)
        .transpose()?;
    let unavailable_reason = details.body.is_none().then(|| {
        details.entry.discovery_error.clone().unwrap_or_else(|| {
            "native source body is unavailable; refresh the focused provider".into()
        })
    });
    Ok(NativeSkillDetails {
        provenance: NativeSkillProvenance {
            source_machine: Some(source_machine),
            locator: details
                .root
                .as_ref()
                .map(|root| root.to_string_lossy().into_owned()),
            provider_version: Some(details.provenance.provider_version),
        },
        entry: details.entry,
        body: details.body,
        files: details.files,
        declared_version: metadata
            .as_ref()
            .and_then(|metadata| metadata.declared_version.clone()),
        compatibility_notes: metadata
            .map(|metadata| metadata.compatibility_notes)
            .unwrap_or_default(),
        unavailable_reason,
    })
}
