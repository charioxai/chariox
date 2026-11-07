//! Every active route of an installed App is a source on the ordinary notification router: one
//! occurrence per event the route accepts. An outside agent follows an App's events directly.
use super::*;
use crate::durable_state::workflow_notifications::{
    naming, NotificationOperation, NotificationOutcome, SourceAdmission,
};
use crate::local::{NotificationSourceKind, WorkflowNotificationSource};
use chariox_app_runtime::app_inbox::InboxRoute;

fn source_of(kernel: &str, route: &InboxRoute) -> WorkflowNotificationSource {
    use sha2::{Digest, Sha256};
    let (name, display_name) = naming::app_event_names(
        &route.event_name,
        route
            .source
            .as_ref()
            .map(|source| source.connection_scope.as_str()),
    );
    WorkflowNotificationSource {
        kind: NotificationSourceKind::AppEvent,
        source_id: format!(
            "app_event_{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    kernel,
                    &route.owner_id,
                    &route.installation_id,
                    &route.route_id
                ))
                .expect("identities serialize")
            )
        ),
        owner_user_id: route.owner_id.clone(),
        kernel_id: kernel.into(),
        session_id: route.installation_id.clone(),
        workflow_id: route.route_id.clone(),
        exit_node_id: route.event_name.clone(),
        enabled: true,
        available: true,
        name,
        exit_ordinal: 0,
        display_name,
    }
}

impl KernelRuntimeOwnedState {
    /// Brings the owner's App event sources in line with the installed Apps: one enabled source
    /// per active route of an active installation, and a disabled one for each that is gone. A
    /// grant keeps following a disabled source's row, flagged unavailable, until it detaches.
    pub(super) fn sync_app_event_notification_sources(
        &self,
        owner: &str,
    ) -> Result<(), DaemonError> {
        let kernel = self.config_projection.snapshot().daemon_id;
        let routes = self
            .durable_state_store
            .owner_app_event_routes(owner)
            .map_err(|error| {
                crate::durable_state::workflow_notifications::error(error.to_string())
            })?;
        let mut live = std::collections::BTreeSet::new();
        for route in &routes {
            let source = source_of(&kernel, route);
            live.insert(source.source_id.clone());
            self.register_app_event_source(source)?;
        }
        let (sources, _, _) = self.durable_state_store.notification_inventory(owner)?;
        for mut gone in sources.into_iter().filter(|source| {
            source.kind == NotificationSourceKind::AppEvent
                && source.enabled
                && !live.contains(&source.source_id)
        }) {
            gone.enabled = false;
            self.register_app_event_source(gone)?;
        }
        Ok(())
    }

    fn register_app_event_source(
        &self,
        source: WorkflowNotificationSource,
    ) -> Result<(), DaemonError> {
        match self
            .durable_state_store
            .notify(NotificationOperation::Register(SourceAdmission {
                source,
                workflow_json: String::new(),
            }))? {
            NotificationOutcome::Source(_) => Ok(()),
            _ => Err(crate::durable_state::workflow_notifications::error(
                "unexpected notification registration result",
            )),
        }
    }
}
