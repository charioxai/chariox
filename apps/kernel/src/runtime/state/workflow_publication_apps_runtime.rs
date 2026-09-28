//! Protocol 366: a client's export of a workflow publication is its deployment
//! preparation. The first one pins the publication's App plan, read from the
//! owner's App set, so later exports — the deployment bind re-verifies its
//! package digest — do not follow App updates.
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::workflow_publication_apps::publication_app_plan;

impl KernelRuntimeState {
    /// Returns the session to persist when a plan was pinned.
    pub(super) async fn pin_workflow_publication_apps(
        &self,
        session_id: &str,
        publication_ref: &str,
        caller_user_id: &str,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        let (publication, snapshot) = {
            let sessions = self.owned.session_store.read();
            let publication =
                sessions.resolve_workflow_publication_ref(session_id, publication_ref)?;
            let snapshot =
                sessions.resolve_workflow_publication_snapshot(session_id, publication.id())?;
            (publication, snapshot)
        };
        // The export reports a missing snapshot; only the owner pins.
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        if publication.apps().is_some() || publication.created_by_user_id() != caller_user_id {
            return Ok(None);
        }
        let grants = crate::workflow_publication_requirements::app_grant_uses(
            &snapshot.workflow,
            &snapshot.agents,
        );
        let store = self.owned.durable_state_store.clone();
        let (owner, session, id) = (
            caller_user_id.to_owned(),
            session_id.to_owned(),
            publication.id().to_owned(),
        );
        let feeding = tokio::task::spawn_blocking(move || {
            store.app_installations_feeding_publication(&owner, &session, &id)
        })
        .await
        .map_err(|_| apps_error("its App automations could not be read"))??;
        if grants.is_empty() && feeding.is_empty() {
            return Ok(None);
        }
        let set = self
            .read_app_set(caller_user_id)
            .await
            .map_err(|code| apps_error(&format!("the App set could not be read ({code:?})")))?;
        let mut entries = Vec::new();
        for installation in set.into_iter().filter(|installation| {
            grants.contains_key(&installation.installation_id)
                || feeding.contains(&installation.installation_id)
        }) {
            let store = self.owned.durable_state_store.clone();
            let (owner, id) = (
                caller_user_id.to_owned(),
                installation.installation_id.clone(),
            );
            let (package_digest, identity) =
                tokio::task::spawn_blocking(move || store.active_app_release_identity(&owner, &id))
                    .await
                    .map_err(|_| apps_error("an App release could not be read"))?
                    .map_err(|_| apps_error("an App release is not active or not trusted"))?;
            if package_digest != installation.release.package_digest {
                return Err(apps_error("an App changed while its plan was read; retry"));
            }
            entries.push((installation, identity));
        }
        let Some(plan) = publication_app_plan(entries, &grants, session_id, publication.id())?
        else {
            return Ok(None);
        };
        self.owned
            .session_store
            .write()
            .pin_workflow_publication_apps(session_id, publication.id(), plan)?;
        self.owned
            .session_snapshot_without_projection_update(session_id)
            .map(Some)
    }
}

fn apps_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "prepare workflow publication Apps",
        message: format!("workflow publication Apps: {message}"),
    }
}
