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
    /// A listing (a leased agent's manifest, the synchronous tool listing):
    /// see `note_listing`.
    pub(crate) fn app_extension_tools_for_agent(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        if !agent
            .extension_grants()
            .iter()
            .any(|grant| grant.kind == ExtensionKind::App)
        {
            return Ok(Vec::new());
        }
        // Seeding reads releases, so it runs under an App admission slot.
        let Ok(permit) = self.try_admit() else {
            // Saturated: an agent whose Apps neither run nor are dormant gets
            // no App tools now, as before seeding; a running App's tools must
            // not silently disappear.
            if self.has_active_apps_for_agent(agent) {
                return Err(crate::runtime::app_worker::AppWorkerError::Busy.into());
            }
            self.note_listing(agent, &BTreeSet::new());
            return Ok(Vec::new());
        };
        self.app_extension_listing_admitted(agent, occupied, &permit)
    }

    /// The tools an agent's provider lists, under the caller's App admission
    /// permit (see `app_extension_tools_for_agent_admitted`); it records which
    /// bound Apps the listing left out (`note_listing`).
    pub(crate) fn app_extension_listing_admitted(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
        _permit: &tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        let (tools, listed) = self.project_for_agent(agent, occupied)?;
        self.note_listing(agent, &listed);
        Ok(tools)
    }

    /// Caller obtained this AppControl service's shared permit before entering
    /// its bounded blocking task. This avoids a second semaphore acquisition.
    /// A dispatch resolving a tool: nothing is listed, so nothing is recorded.
    pub(crate) fn app_extension_tools_for_agent_admitted(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
        _permit: &tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Vec<RemoteExtensionTool>, AppToolsError> {
        Ok(self.project_for_agent(agent, occupied)?.0)
    }

    /// The agent's App tools and the installations they come from. The caller
    /// holds an App admission slot.
    fn project_for_agent(
        &self,
        agent: &AgentInstance,
        occupied: &BTreeSet<String>,
    ) -> Result<(Vec<RemoteExtensionTool>, BTreeSet<String>), AppToolsError> {
        self.seed_bound_dormant(agent);
        let leases = self.bound_app_leases(agent);
        let dormant = self.bound_dormant_catalogs(agent);
        let catalogs = leases
            .iter()
            .map(|lease| lease.catalog().clone())
            .chain(dormant.iter().cloned())
            .collect::<Vec<_>>();
        if catalogs.is_empty() {
            return Ok(Default::default());
        }
        let current = self
            .store
            .current_app_catalogs(agent.owner_user_id(), &catalogs)?;
        self.project_app_tools(current, leases, &dormant, occupied)
    }

    /// A listing showed the agent the bound Apps in `listed`: the others are
    /// remembered, so the agent's catalog is refreshed once one starts; a
    /// listed App needs no refresh.
    pub(crate) fn note_listing(&self, agent: &AgentInstance, listed: &BTreeSet<String>) {
        for grant in agent.extension_grants() {
            if grant.kind != ExtensionKind::App {
                continue;
            }
            if listed.contains(&grant.name) {
                self.workers
                    .forget_unlisted(agent.owner_user_id(), &grant.name, agent.id());
            } else {
                self.workers
                    .note_unlisted(agent.owner_user_id(), &grant.name, agent.id());
            }
        }
    }

    /// A revoked binding needs no refresh when its App starts.
    pub(crate) fn forget_unlisted_binding(&self, agent: &AgentInstance, installation: &str) {
        self.workers
            .forget_unlisted(agent.owner_user_id(), installation, agent.id());
    }

    /// A binding saved while its App neither runs nor is dormant lists no
    /// tools yet: the agent's catalog is refreshed once the App starts.
    pub(crate) fn note_unlisted_binding(&self, agent: &AgentInstance, installation: &str) {
        let owner = agent.owner_user_id();
        if self.active_app_lease(owner, installation).is_none()
            && !self.is_app_dormant(owner, installation)
        {
            self.workers.note_unlisted(owner, installation, agent.id());
        }
    }

    #[cfg(test)]
    pub(crate) fn app_unlisted(&self, owner: &str, installation: &str, agent: &str) -> bool {
        self.workers.is_unlisted(owner, installation, agent)
    }

    /// Agents due a catalog refresh because an App their listing left out
    /// has started since.
    pub(crate) fn take_started_app_refreshes(&self) -> Vec<String> {
        self.workers.take_due()
    }

    /// Notified when an App starts that an agent's listing left out.
    pub(crate) fn started_app_refreshes_signal(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.workers.due_signal()
    }

    /// A bound App that neither runs nor is dormant, but may start on demand,
    /// gets a dormant catalog from its verified active release: its tools are
    /// listed before the first call starts it, also after a kernel restart. A
    /// user stop, a failed generation or a revoked publisher keeps it unlisted.
    /// The caller holds an App admission slot.
    pub(crate) fn seed_bound_dormant(&self, agent: &AgentInstance) {
        for grant in agent.extension_grants() {
            if grant.kind == ExtensionKind::App
                && agent.has_extension_grant(ExtensionKind::App, &grant.name)
            {
                self.seed_dormant(agent.owner_user_id(), &grant.name);
            }
        }
    }

    /// One installation of `seed_bound_dormant`. The dormant catalog is the
    /// owner's; only agents bound to the installation list it. The caller
    /// holds an App admission slot.
    pub(crate) fn seed_dormant(&self, owner: &str, installation: &str) {
        if self.active_app_lease(owner, installation).is_some()
            || self.is_app_dormant(owner, installation)
            || !matches!(
                self.store.app_worker_start_gate(owner, installation),
                Ok(crate::durable_state::app_worker_lifecycle::StartGate::Allowed)
            )
        {
            return;
        }
        if let Ok(catalog) = self.store.active_app_event_catalog(owner, installation) {
            super::AppWorkerPublisher::new(self.workers.clone(), self.event_pump.clone())
                .retain_dormant(owner, catalog);
        }
    }

    pub(crate) fn has_active_apps_for_agent(&self, agent: &AgentInstance) -> bool {
        !self.bound_app_leases(agent).is_empty() || !self.bound_dormant_catalogs(agent).is_empty()
    }

    fn bound_dormant_catalogs(
        &self,
        agent: &AgentInstance,
    ) -> Vec<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>> {
        self.dormant_app_catalogs(agent.owner_user_id())
            .into_iter()
            .filter(|catalog| {
                agent.has_extension_grant(ExtensionKind::App, catalog.installation_id())
            })
            .collect()
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

    /// The tools, and the installations whose catalog was listed.
    fn project_app_tools(
        &self,
        current: Vec<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>>,
        leases: Vec<crate::runtime::app_worker::AppWorkerLease>,
        dormant: &[std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>],
        occupied: &BTreeSet<String>,
    ) -> Result<(Vec<RemoteExtensionTool>, BTreeSet<String>), AppToolsError> {
        let mut names = occupied.clone();
        let mut output = Vec::new();
        let mut listed = BTreeSet::new();
        let mut encoded = EncodingBudget(0);
        for catalog in current {
            if !leases.iter().any(|lease| {
                std::sync::Arc::ptr_eq(lease.catalog(), &catalog) && !lease.is_stopped()
            }) && !dormant
                .iter()
                .any(|stopped| std::sync::Arc::ptr_eq(stopped, &catalog))
            {
                continue;
            }
            listed.insert(catalog.installation_id().to_owned());
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
        Ok((output, listed))
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
