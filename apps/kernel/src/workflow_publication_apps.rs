//! Protocol 366: a workflow publication's App plan
//! (`chariox.publication-apps.v1`). It lists the Apps the workflow uses —
//! granted to its agents or feeding its publication through an App
//! automation — with the exact release, the signer, the approved capabilities
//! digest and the configuration a deployed copy reproduces: grants by agent,
//! automations, inbox routes and connection grants. It names the generator
//! connections the Apps use but carries no App data and no secret.
use std::collections::BTreeMap;

use crate::local::{AppAutomationStatus, AppSetInstallation};
use crate::DaemonError;

pub(crate) const PUBLICATION_APPS_SCHEMA: &str = "chariox.publication-apps.v1";

/// The signer and approval of an installation's active release.
#[derive(Debug, Clone)]
pub(crate) struct ReleaseIdentity {
    pub(crate) capabilities_digest: String,
    pub(crate) publisher_key_id: String,
    pub(crate) publisher_key_fingerprint: String,
}

/// The plan of the Apps in `set` that the publication uses; `None` when it
/// uses no App. A granted installation missing from the set fails: the owner
/// must not deploy a workflow whose App is not installed.
pub(crate) fn publication_app_plan(
    set: Vec<(AppSetInstallation, ReleaseIdentity)>,
    grants: &BTreeMap<String, Vec<serde_json::Value>>,
    session_id: &str,
    publication_id: &str,
) -> Result<Option<serde_json::Value>, DaemonError> {
    let mut apps = Vec::new();
    for (installation, identity) in set {
        let automations = installation
            .automations
            .iter()
            .filter(|automation| {
                automation.session_id == session_id
                    && automation.publication_id == publication_id
                    && automation.status == AppAutomationStatus::Active
            })
            .map(|automation| {
                serde_json::json!({
                    "automation_id": automation.automation_id,
                    "event_name": automation.event_name,
                    "event_version": automation.event_version,
                    "endpoint_id": automation.endpoint_id,
                    "queue_id": automation.queue_id,
                    "scheduled": automation.scheduled,
                    "delivery_mode": automation.delivery_mode,
                })
            })
            .collect::<Vec<_>>();
        let uses = grants.get(&installation.installation_id);
        if uses.is_none() && automations.is_empty() {
            continue;
        }
        // A route paused while a deployment copy holds its interest is still
        // the owner's route.
        let inbox_routes = installation
            .inbox_routes
            .iter()
            .map(|route| {
                serde_json::json!({
                    "route_id": route.route_id,
                    "event_name": route.event_name,
                    "source_event_type": route.source_event_type,
                    "source_event_version": route.source_event_version,
                    "connection": route.connection.as_ref().map(|connection| serde_json::json!({
                        "generator_id": connection.generator_id,
                        "connection_id": connection.connection_id,
                        "connection_scope": connection.connection_scope,
                    })),
                })
            })
            .collect::<Vec<_>>();
        let connections = installation
            .connections
            .iter()
            .map(|connection| {
                serde_json::json!({
                    "generator_id": connection.generator_id,
                    "connection_id": connection.connection_id,
                    "actions": connection.actions,
                })
            })
            .collect::<Vec<_>>();
        apps.push(serde_json::json!({
            "installation_id": installation.installation_id,
            "app_id": installation.app_id,
            "version": installation.release.version,
            "publisher_id": installation.release.publisher_id,
            "publisher_key_id": identity.publisher_key_id,
            "publisher_key_fingerprint": identity.publisher_key_fingerprint,
            "package_digest": installation.release.package_digest,
            "schema_version": installation.release.schema_version,
            "capabilities_digest": identity.capabilities_digest,
            "grants": uses.cloned().unwrap_or_default(),
            "automations": automations,
            "inbox_routes": inbox_routes,
            "connections": connections,
        }));
    }
    if let Some(missing) = grants.keys().find(|installation| {
        !apps
            .iter()
            .any(|app| app["installation_id"] == installation.as_str())
    }) {
        return Err(DaemonError::LocalTransport {
            operation: "prepare workflow publication Apps",
            message: format!(
                "App installation `{missing}` is granted to a workflow agent but is not installed"
            ),
        });
    }
    Ok((!apps.is_empty()).then(|| {
        serde_json::json!({
            "schema": PUBLICATION_APPS_SCHEMA,
            "apps": apps,
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_app_missing_from_the_set_fails_and_no_app_means_no_plan() {
        let grants = BTreeMap::from([(
            "todo".to_string(),
            vec![serde_json::json!({"agent_id": "agent-1", "node_ids": ["node-1"]})],
        )]);
        let error = publication_app_plan(Vec::new(), &grants, "session", "publication")
            .expect_err("a granted App must be installed");
        assert!(error.to_string().contains("`todo`"));
        assert!(
            publication_app_plan(Vec::new(), &BTreeMap::new(), "session", "publication")
                .unwrap()
                .is_none()
        );
    }
}
