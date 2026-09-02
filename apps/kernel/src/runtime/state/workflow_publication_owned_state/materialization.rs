use super::*;

pub(super) fn normalized_runtime_key(value: Option<&str>) -> Result<Option<String>, DaemonError> {
    value
        .map(|value| {
            let key = value.trim();
            if key.is_empty() || key.len() > 200 || key.chars().any(char::is_control) {
                return Err(materialization_error("publication runtime key is invalid"));
            }
            Ok(key.to_string())
        })
        .transpose()
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_resume_publication_materialization(
        &self,
        runtime_key: &str,
        publication_id: &str,
        snapshot_digest: &str,
        snapshot: &crate::session::WorkflowPublicationSnapshot,
        owner_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        let mut matching = self
            .session_store
            .read()
            .list_all_sessions()
            .into_iter()
            .filter_map(|session| {
                if session.owner_user_id() != owner_user_id {
                    return None;
                }
                session
                    .workflow_publications()
                    .iter()
                    .find(|publication| {
                        publication.created_by_user_id() == owner_user_id
                            && publication
                                .runtime_materialization()
                                .is_some_and(|binding| binding.key == runtime_key)
                    })
                    .cloned()
                    .map(|publication| (session, publication))
            });
        let Some((session, publication)) = matching.next() else {
            return Ok(None);
        };
        if matching.next().is_some() {
            return Err(materialization_error(
                "publication runtime key has multiple owners",
            ));
        }
        if publication.id() != publication_id {
            return Err(materialization_error(
                "publication runtime key is already bound to a different publication",
            ));
        }
        if !session.is_hidden()
            || session.status() == crate::session::SessionStatus::Ended
            || !publication.enabled()
        {
            return Err(materialization_error(
                "publication runtime is no longer resumable",
            ));
        }
        let binding = publication
            .runtime_materialization()
            .ok_or_else(|| materialization_error("publication runtime binding is missing"))?;
        for agent_id in binding.agent_id_map.values() {
            let agent = self.agent_store.get_agent(agent_id)?;
            if agent.session_id() != session.id() || agent.owner_user_id() != owner_user_id {
                return Err(materialization_error(
                    "publication runtime agent ownership changed",
                ));
            }
        }
        if publication.source_snapshot_digest() != Some(snapshot_digest) {
            self.workflow_reconfigure_publication_runtime(session.id(), &publication, snapshot)?;
        }
        Ok(Some(LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: publication_id.to_string(),
            session: self.workflow_session(session.id())?,
            agent_id_map: binding.agent_id_map.clone(),
        }))
    }
}

pub(super) fn materialization_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "materialize workflow publication",
        message: message.to_string(),
    }
}
