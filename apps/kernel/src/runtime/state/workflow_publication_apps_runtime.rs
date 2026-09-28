//! Protocol 368: a client's export of a workflow publication is a deployment
//! release's preparation. Each owner export packages the plan read from the
//! owner's current App set, so a new release follows App updates while an
//! existing release keeps the plan it was exported with.
use std::collections::BTreeMap;

use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::workflow_publication_apps::publication_app_plan;

impl KernelRuntimeState {
    /// The App plan the owner's export packages; `None` when the caller is not
    /// the owner or the workflow uses no App.
    pub(super) async fn workflow_publication_apps_for_export(
        &self,
        session_id: &str,
        publication_ref: &str,
        caller_user_id: &str,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let (publication, snapshot) = {
            let sessions = self.owned.session_store.read();
            let publication =
                sessions.resolve_workflow_publication_ref(session_id, publication_ref)?;
            let snapshot =
                sessions.resolve_workflow_publication_snapshot(session_id, publication.id())?;
            (publication, snapshot)
        };
        // The export reports a missing snapshot; only the owner prepares.
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        if publication.created_by_user_id() != caller_user_id {
            return Ok(None);
        }
        Ok(self
            .read_publication_app_plan(&publication, &snapshot, caller_user_id)
            .await?
            .map(|(plan, _)| plan))
    }
}

impl KernelRuntimeState {
    /// The publication's App plan read from the owner's current App set, with
    /// each planned installation's signed capabilities; `None` without Apps.
    pub(super) async fn read_publication_app_plan(
        &self,
        publication: &crate::session::WorkflowPublicationDefinition,
        snapshot: &crate::session::WorkflowPublicationSnapshot,
        owner: &str,
    ) -> Result<Option<(serde_json::Value, BTreeMap<String, serde_json::Value>)>, DaemonError> {
        let grants = crate::workflow_publication_requirements::app_grant_uses(
            &snapshot.workflow,
            &snapshot.agents,
        );
        let store = self.owned.durable_state_store.clone();
        let (feeding_owner, session, id) = (
            owner.to_owned(),
            publication.session_id().to_owned(),
            publication.id().to_owned(),
        );
        let feeding = tokio::task::spawn_blocking(move || {
            store.app_installations_feeding_publication(&feeding_owner, &session, &id)
        })
        .await
        .map_err(|_| apps_error("its App automations could not be read"))??;
        if grants.is_empty() && feeding.is_empty() {
            return Ok(None);
        }
        let set = self
            .read_app_set(owner)
            .await
            .map_err(|code| apps_error(&format!("the App set could not be read ({code:?})")))?;
        // A release packages the Apps the workflow uses as installed now.
        if let Some(missing) = grants
            .keys()
            .find(|granted| !set.iter().any(|app| &app.installation_id == *granted))
        {
            return Err(apps_error(&format!(
                "App installation `{missing}` that the workflow uses is not installed; reinstall it or remove the grant"
            )));
        }
        let mut entries = Vec::new();
        let mut capabilities = BTreeMap::new();
        for installation in set.into_iter().filter(|installation| {
            grants.contains_key(&installation.installation_id)
                || feeding.contains(&installation.installation_id)
        }) {
            let store = self.owned.durable_state_store.clone();
            let (identity_owner, id) = (owner.to_owned(), installation.installation_id.clone());
            let (package_digest, identity) = tokio::task::spawn_blocking(move || {
                store.active_app_release_identity(&identity_owner, &id)
            })
            .await
            .map_err(|_| apps_error("an App release could not be read"))?
            .map_err(|_| apps_error("an App release is not active or not trusted"))?;
            if package_digest != installation.release.package_digest {
                return Err(apps_error("an App changed while its plan was read; retry"));
            }
            capabilities.insert(
                installation.installation_id.clone(),
                installation.capabilities.clone(),
            );
            entries.push((installation, identity));
        }
        Ok(
            publication_app_plan(entries, &grants, publication.session_id(), publication.id())?
                .map(|plan| (plan, capabilities)),
        )
    }
}

fn apps_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "prepare workflow publication Apps",
        message: format!("workflow publication Apps: {message}"),
    }
}
