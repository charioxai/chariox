//! One current App tool projection for ordinary, Meta and remote/native agents.
//! The same installation binding is intersected with actual activated owners;
//! the metadata grants no authority to a later invocation.

use super::AppControlService;
use crate::{
    agent::AgentInstance,
    durable_state::app_tools::AppToolsError,
    extension::{
        ExtensionAuthority, ExtensionDefinitionOrigin, ExtensionExecutionLocation, ExtensionKind,
        RemoteExtensionTool,
    },
};
use chariox_app_runtime::app_catalog::CatalogError;
use std::{collections::BTreeSet, io};

const MAX_TOOLS: usize = 1024;
const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;

impl AppControlService {
    /// `agent` and the occupied runtime namespace come from the existing kernel
    /// stores, never App request data. All callers share this projection instead
    /// of creating an MCP server or a second remote installation registry.
    pub(crate) fn app_extension_tools_for_agent(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        if !self.has_active_apps_for_agent(agent) {
            return Ok(Vec::new());
        }
        let permit = self
            .try_admit()
            .map_err(|_| crate::runtime::app_worker::AppWorkerError::Busy)?;
        self.app_extension_tools_for_agent_admitted(agent, occupied, &permit)
    }

    /// Caller obtained this AppControl service's shared permit before entering
    /// its bounded blocking task. This avoids a second semaphore acquisition.
    pub(crate) fn app_extension_tools_for_agent_admitted(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
        _permit: &tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        let leases = self.bound_app_leases(agent);
        let catalogs = leases
            .iter()
            .map(|lease| lease.catalog().clone())
            .collect::<Vec<_>>();
        if catalogs.is_empty() {
            return Ok(Vec::new());
        }
        let current = self
            .store
            .current_app_catalogs(agent.owner_user_id(), &catalogs)?;
        self.project_app_tools(current, leases, occupied)
    }

    pub(crate) fn has_active_apps_for_agent(&self, agent: &AgentInstance) -> bool {
        !self.bound_app_leases(agent).is_empty()
    }

    fn bound_app_leases(
        &self,
        agent: &AgentInstance,
    ) -> Vec<crate::runtime::app_worker::AppWorkerLease> {
        let mut leases = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        // The actual owner registry contains at most 64 projections. Four bounded
        // pages are sufficient; unknown/gratuitous grant names allocate no lookup.
        for _ in 0..4 {
            let page =
                self.active_app_leases(cursor.as_ref().map(|(o, i)| (o.as_str(), i.as_str())), 16);
            let full = page.len() == 16;
            cursor = page.last().map(|lease| {
                (
                    lease.owner().to_owned(),
                    lease.catalog().installation_id().to_owned(),
                )
            });
            leases.extend(page.into_iter().filter(|lease| {
                lease.owner() == agent.owner_user_id()
                    && agent
                        .has_extension_grant(ExtensionKind::App, lease.catalog().installation_id())
            }));
            if !full {
                break;
            }
        }
        leases
    }

    fn project_app_tools(
        &self,
        current: Vec<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>>,
        leases: Vec<crate::runtime::app_worker::AppWorkerLease>,
        occupied: &BTreeSet<String>,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        let mut names = occupied.clone();
        let mut output = Vec::new();
        let mut encoded = EncodingBudget(0);
        for catalog in current {
            if !leases.iter().any(|lease| {
                std::sync::Arc::ptr_eq(lease.catalog(), &catalog) && !lease.is_stopped()
            }) {
                continue;
            }
            for tool in catalog.app_catalog().tools() {
                if tool
                    .action
                    .as_ref()
                    .and_then(|action| action.critical_validation.as_ref())
                    .is_some()
                {
                    continue;
                }
                if !names.insert(tool.name.clone()) {
                    return Err(CatalogError::NameCollision.into());
                }
                if output.len() == MAX_TOOLS {
                    return Err(CatalogError::Limit.into());
                }
                let value = RemoteExtensionTool {
                    kind: ExtensionKind::App,
                    name: catalog.installation_id().into(),
                    tool_name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    authority: ExtensionAuthority::Home,
                    definition_origin: ExtensionDefinitionOrigin::Home,
                    execution_location: ExtensionExecutionLocation::Home,
                    safety: None,
                    timeout_sec: Some(30),
                    version_hash: Some(format!(
                        "{}:{}",
                        catalog.generation(),
                        catalog.app_catalog().catalog_digest()
                    )),
                };
                serde_json::to_writer(&mut encoded, &value).map_err(|_| CatalogError::Limit)?;
                output.push(value);
            }
        }
        Ok(output)
    }
}

struct EncodingBudget(usize);
impl io::Write for EncodingBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("catalog bound"))?;
        if next > MAX_ENCODED_BYTES {
            return Err(io::Error::other("catalog bound"));
        }
        self.0 = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
