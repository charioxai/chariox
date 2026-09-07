use super::*;
use crate::durable_state::apps::AppRegistryMutation;
use crate::local::{AppInstallationRequest, ListAppInstallationsRequest, LocalDaemonResponse};
use chariox_app_runtime::installation::ReleaseMetadata;

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-relay-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }

    fn config(&self) -> crate::DaemonConfig {
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config.state.path = Some(self.0.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(self.0.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(self.0.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(self.0.join("artifacts.db").display().to_string());
        config.session_history_root_default = self.0.join("sessions");
        config
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn caller(owner: &str) -> RelayCallerIdentity {
    RelayCallerIdentity {
        realm_id: "test".into(),
        subject: format!("client-{owner}"),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(owner.into()),
        public_key_thumbprint: None,
    }
}

async fn dispatch(
    router: &CommandRouter,
    cache: &CommandResultCache,
    owner: Option<&str>,
    request: LocalDaemonRequest,
    command_id: &str,
) -> LocalDaemonResponse {
    let result = dispatch_relay_client_request(
        router,
        &AtomicU64::new(1),
        owner.map(caller),
        request,
        Some(command_id.into()),
        cache,
    )
    .await;
    match result {
        RelayDispatchOutcome::Response(value) => serde_json::from_value(value).unwrap(),
        RelayDispatchOutcome::RelayError(_) => panic!("unexpected relay transport error"),
    }
}

#[tokio::test]
async fn app_relay_replays_reauthorize_the_owner_and_read_current_installations() {
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let list = LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
        after: None,
        limit: None,
    });
    let empty = LocalDaemonResponse::AppInstallationsListed {
        installations: vec![],
        next_cursor: None,
    };
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), list.clone(), "list").await,
        empty
    );
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::CreateAndStage {
                installation_id: "todo-alice".into(),
                release: ReleaseMetadata {
                    app_id: "com.chariox.todo".into(),
                    version: "1.0.0".into(),
                    publisher_id: "publisher".into(),
                    package_digest: format!("sha256:{:064x}", 1),
                    schema_version: 1,
                    capabilities_digest: format!("sha256:{:064x}", 2),
                    catalog_digest: format!("sha256:{:064x}", 3),
                    view_digest: format!("sha256:{:064x}", 4),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    let current = dispatch(&router, &cache, Some("alice"), list.clone(), "list").await;
    assert!(
        matches!(current, LocalDaemonResponse::AppInstallationsListed { installations, .. } if installations.len() == 1)
    );
    // Same command ID/body reaches authorization, even after Alice's success.
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), list.clone(), "list").await,
        empty
    );
    assert!(matches!(
        dispatch(&router, &cache, None, list, "list").await,
        LocalDaemonResponse::AppRequestFailed {
            code: crate::local::AppRequestErrorCode::Unauthorized
        }
    ));
    for request in [
        LocalDaemonRequest::GetAppInstallation(AppInstallationRequest {
            installation_id: "todo-alice".into(),
        }),
        LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
            installation_id: "todo-alice".into(),
        }),
    ] {
        let own = dispatch(&router, &cache, Some("alice"), request.clone(), "same-id").await;
        assert!(!matches!(own, LocalDaemonResponse::AppRequestFailed { .. }));
        assert!(matches!(
            dispatch(&router, &cache, Some("bob"), request, "same-id").await,
            LocalDaemonResponse::AppRequestFailed {
                code: crate::local::AppRequestErrorCode::NotFound
            }
        ));
    }
}
