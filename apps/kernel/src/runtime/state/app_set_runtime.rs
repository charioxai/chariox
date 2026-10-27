//! Protocol 361: the owner's App set, a versioned description of each active
//! installation — its release, the signed capabilities the owner approved,
//! and its configuration (automations, inbox routes, connection grants). A
//! kernel copy (Phase 2) installs from it; App data is never part of it. It is
//! read through the same owner-scoped requests the clients use. It fails
//! closed: an installation that cannot be read completely (an update in
//! between, a revoked publisher, an unavailable archive) fails the whole set,
//! because a copy must not pair a release with another release's
//! capabilities. Configuration is read as it stands; a concurrent owner edit
//! may or may not be included.
use super::KernelRuntimeState;
use crate::local::{
    AppRequestErrorCode, AppSetInstallation, AppWorkerRequest, LocalDaemonRequest,
    LocalDaemonResponse,
};
use crate::runtime::command::KernelCommand;

/// Installations read into one App set.
const MAX_INSTALLATIONS: usize = 1000;

impl KernelRuntimeState {
    pub(super) async fn app_set(&self, command: &KernelCommand) -> LocalDaemonResponse {
        let set = match crate::runtime::app_control::owner(command) {
            Ok(owner) => self.read_app_set(&owner).await,
            Err(code) => Err(code),
        };
        match set {
            Ok(installations) => LocalDaemonResponse::AppSet {
                schema: crate::local::APP_SET_SCHEMA.to_owned(),
                installations,
            },
            Err(code) => LocalDaemonResponse::AppRequestFailed { code },
        }
    }

    /// The authenticated owner's App set.
    pub(super) async fn read_app_set(
        &self,
        owner: &str,
    ) -> Result<Vec<AppSetInstallation>, AppRequestErrorCode> {
        let mut active = Vec::new();
        let mut after = None;
        loop {
            let (store, list_owner, cursor) = (
                self.owned.durable_state_store.clone(),
                owner.to_owned(),
                after.take(),
            );
            let permit = self.app_control().try_admit()?;
            let page = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                store.list_app_installations(&list_owner, cursor.as_deref(), 100)
            })
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(crate::runtime::app_control::registry_error)?;
            active.extend(page.installations.into_iter().filter_map(|installation| {
                let installation = crate::runtime::app_control::installation_summary(installation);
                let release = installation.active_release?;
                Some((installation.installation_id, installation.app_id, release))
            }));
            if active.len() > MAX_INSTALLATIONS {
                return Err(AppRequestErrorCode::LimitExceeded);
            }
            match page.next_cursor {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        let store = self.owned.durable_state_store.clone();
        let deployments_owner = owner.to_owned();
        let deployments = tokio::task::spawn_blocking(move || {
            store.app_installation_deployments(&deployments_owner)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
        let mut set = Vec::with_capacity(active.len());
        for (installation_id, app_id, release) in active {
            let read = |request: LocalDaemonRequest| {
                self.app_control_response(owner.to_owned(), installation_id.clone(), request)
            };
            let installation = || AppWorkerRequest {
                installation_id: installation_id.clone(),
            };
            let LocalDaemonResponse::AppAutomations { automations, .. } =
                read(LocalDaemonRequest::ListAppAutomations(installation())).await?
            else {
                return Err(AppRequestErrorCode::StorageUnavailable);
            };
            let LocalDaemonResponse::AppInboxRoutes { routes, .. } =
                read(LocalDaemonRequest::ListAppInboxRoutes(installation())).await?
            else {
                return Err(AppRequestErrorCode::StorageUnavailable);
            };
            let LocalDaemonResponse::AppConnections { connections, .. } =
                read(LocalDaemonRequest::ListAppConnections(installation())).await?
            else {
                return Err(AppRequestErrorCode::StorageUnavailable);
            };
            let store = self.owned.durable_state_store.clone();
            let (capability_owner, capability_installation) =
                (owner.to_owned(), installation_id.clone());
            let permit = self.app_control().try_admit()?;
            let (digest, capabilities) = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                store.active_app_capabilities(&capability_owner, &capability_installation)
            })
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(AppRequestErrorCode::from)?;
            // One generation per entry: an update that committed while the
            // set was read would pair one release with another's capabilities.
            if digest != release.package_digest {
                return Err(AppRequestErrorCode::Conflict);
            }
            set.push(AppSetInstallation {
                deployment_id: deployments.get(&installation_id).cloned(),
                installation_id,
                app_id,
                release,
                capabilities,
                automations,
                inbox_routes: routes,
                connections,
            });
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use crate::local::{GetAppSetRequest, LocalDaemonRequest, LocalDaemonResponse};

    #[test]
    fn an_app_set_describes_each_active_installation_of_its_owner_only() {
        let root =
            std::env::temp_dir().join(format!("chariox-app-set-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let harness = crate::local::test_support::LocalRouterTestHarness::with_config(config);
        let store = harness.with_app(|app| app.durable_state_store());
        // Installs `installed` (com.example.state) for alice, with its release.
        crate::durable_state::app_state::fixture_event_catalog(&store);
        let (bytes, publisher) = crate::durable_state::app_state::fixture_event_package();
        let verified = chariox_app_package::verify(
            &bytes,
            &chariox_app_package::VerificationPolicy::new(
                crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                vec![publisher],
            ),
        )
        .unwrap();
        chariox_app_runtime::release_store::ReleaseStore::open_or_create(store.path())
            .unwrap()
            .stage(
                &verified,
                &bytes,
                chariox_app_runtime::release_store::StageBudget {
                    max_stage_bytes: 1024 * 1024,
                    reserved_bytes: 1024 * 1024,
                    host_reserve_bytes: 1024 * 1024,
                },
            )
            .unwrap();
        let set = |user: &str| match harness
            .dispatch_as_user(user, LocalDaemonRequest::GetAppSet(GetAppSetRequest {}))
            .unwrap()
        {
            LocalDaemonResponse::AppSet {
                schema,
                installations,
            } => (schema, installations),
            response => panic!("unexpected response: {response:?}"),
        };
        let (schema, installations) = set("alice");
        assert_eq!(schema, "chariox.app-set.v1");
        assert_eq!(installations.len(), 1);
        let installation = &installations[0];
        assert_eq!(
            (
                installation.installation_id.as_str(),
                installation.app_id.as_str()
            ),
            ("installed", "com.example.state")
        );
        assert!(installation.release.package_digest.starts_with("sha256:"));
        assert!(installation.capabilities.is_object());
        assert!(installation.automations.is_empty());
        assert!(installation.inbox_routes.is_empty());
        assert!(installation.connections.is_empty());
        // Another owner's set does not include it.
        assert!(set("bob").1.is_empty());
        drop(harness);
        let _ = std::fs::remove_dir_all(root);
    }
}
