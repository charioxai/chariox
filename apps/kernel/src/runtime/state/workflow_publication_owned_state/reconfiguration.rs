//! Destination profile changes are permitted only during retained-runtime boot
//! preparation. Graph, grants, history, queue and schedule identities stay fixed.

use super::*;
use materialization::materialization_error;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_reconfigure_publication_runtime(
        &self,
        session_id: &str,
        publication: &crate::session::WorkflowPublicationDefinition,
        snapshot: &crate::session::WorkflowPublicationSnapshot,
    ) -> Result<(), DaemonError> {
        if self
            .config_projection
            .snapshot()
            .publication_control_state_root
            .is_none()
            || self.publication_activation.is_active()
        {
            return Err(materialization_error(
                "publication profile changes require a controlled restart before activation",
            ));
        }
        self.durable_state_store.with_workflow_runtime_transition_lock(|| {
            let mut session = self.session_snapshot_without_projection_update(session_id)?;
            let previous = session
                .workflow_publication_snapshot(publication.id())
                .ok_or_else(|| materialization_error("retained publication snapshot is missing"))?;
            let mut normalized = snapshot.clone();
            for agent in &mut normalized.agents {
                let old = previous
                    .agents
                    .iter()
                    .find(|old| old.id() == agent.id())
                    .ok_or_else(|| {
                        materialization_error(
                            "publication profile change cannot replace source agents",
                        )
                    })?;
                agent.set_provider(old.provider());
                agent.set_model(old.model().map(str::to_string));
                agent.set_effort(old.effort().map(str::to_string));
                agent.set_account_profile(old.account_profile().map(str::to_string));
            }
            if normalized != *previous {
                return Err(materialization_error(
                    "publication runtime snapshot may change only provider, account, model and effort",
                ));
            }
            if session.has_any_active_prompt() || session.has_active_workflow_run() {
                return Err(materialization_error(
                    "publication profile change requires active work to drain before restart",
                ));
            }
            let binding = publication
                .runtime_materialization()
                .ok_or_else(|| materialization_error("publication runtime binding is missing"))?;
            let mut profiles = BTreeMap::new();
            for source in &snapshot.agents {
                let runtime_id = binding
                    .agent_id_map
                    .get(source.id())
                    .ok_or_else(|| {
                        materialization_error("publication agent mapping is incomplete")
                    })?;
                profiles.insert(runtime_id.clone(), source);
            }
            // Pool agents are independent identities, but inherit the effective
            // profile of their source node on the same controlled restart.
            for instance in session.workflow_runtime_instances() {
                if instance.workflow_id() != snapshot.workflow.id() {
                    continue;
                }
                for (node_id, agent_id) in instance.node_agent_ids() {
                    let node = snapshot
                        .workflow
                        .nodes()
                        .iter()
                        .find(|node| node.id() == node_id)
                        .ok_or_else(|| {
                            materialization_error("retained pool node is missing")
                        })?;
                    let source = snapshot
                        .agents
                        .iter()
                        .find(|agent| agent.id() == node.agent_id())
                        .ok_or_else(|| {
                            materialization_error("retained pool source agent is missing")
                        })?;
                    if profiles
                        .insert(agent_id.clone(), source)
                        .is_some_and(|existing| existing.id() != source.id()) {
                        return Err(materialization_error(
                            "retained pool agent has conflicting source nodes",
                        ));
                    }
                }
            }
            let mut agents = session.agents().to_vec();
            for (agent_id, source) in profiles {
                if self
                    .provider_store
                    .get_run_for_agent(session.id(), &agent_id)
                    .is_some_and(|run| run.state() != crate::provider::ProviderRunState::Ended)
                {
                    return Err(materialization_error(
                        "publication profile change requires provider processes to stop",
                    ));
                }
                let agent = agents
                    .iter_mut()
                    .find(|agent| agent.id() == agent_id)
                    .ok_or_else(|| {
                        materialization_error("retained publication agent is missing")
                    })?;
                if agent.session_id() != session.id()
                    || agent.owner_user_id() != session.owner_user_id()
                {
                    return Err(materialization_error(
                        "retained publication agent ownership changed",
                    ));
                }
                let changes_context = agent.provider() != source.provider()
                    || agent.model() != source.model()
                    || agent.effort() != source.effort()
                    || agent.account_profile() != source.account_profile()
                    || agent.active_substitute_index().is_some();
                if agent.active_substitute_index().is_some() {
                    agent.deactivate_substitute();
                }
                agent.set_provider(source.provider());
                agent.set_model(source.model().map(str::to_string));
                agent.set_effort(source.effort().map(str::to_string));
                agent.set_account_profile(source.account_profile().map(str::to_string));
                agent.set_primary_profile(
                    source.provider(),
                    source.model().map(str::to_string),
                    source.effort().map(str::to_string),
                );
                if changes_context {
                    agent.set_provider_resume_state(Default::default());
                }
            }
            session.set_agents(agents);
            session
                .replace_publication_runtime_configuration(publication.id(), snapshot.clone())
                .map_err(|error| materialization_error(&error))?;
            // Persist before publishing any changed agent or session in memory.
            // This transaction includes both event replay and normalized state.
            self.durable_state_store
                .persist_publication_runtime_configuration(&session)?;
            {
                let mut agents = self.agent_store.write();
                for agent in session.agents() {
                    agents.restore_agent(agent.clone());
                }
            }
            self.session_store
                .commit_publication_runtime_configuration(session.clone())?;
            self.session_projection.update(session);
            self.runtime_projection_changes.record_change();
            Ok(())
        })
    }
}
