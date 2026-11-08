use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::managed_context::outbound_service::{
    start_managed_context_outbound_operation, ManagedContextOutboundCaller,
    ManagedContextOutboundOperationStore,
};
use crate::transport::relay_client::RelayClientState;

/// Who asked for a transfer, and from which session.
#[derive(Clone, Copy)]
pub(crate) struct OutboundRequester<'a> {
    pub user_id: &'a str,
    pub session_id: Option<&'a str>,
}

pub(crate) fn execute_managed_context_outbound_request(
    config: DaemonConfig,
    relay_state: Arc<RwLock<RelayClientState>>,
    store: ManagedContextOutboundOperationStore,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    runtime: crate::runtime::state::KernelRuntimeState,
    requester: OutboundRequester<'_>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let caller_user_id = requester.user_id;
    authorize_source_user(&config, caller_user_id)?;
    match request {
        LocalDaemonRequest::StartManagedContextTransfer(request) => {
            let request = *request;
            let ticket = match (request.ticket, request.owner_managed) {
                (Some(ticket), None)
                    if ticket.context_plan.package_binding().destination.is_none() =>
                {
                    ticket
                }
                (None, Some(selection)) if request.interactive => {
                    // Refuse an event connection that cannot be copied before the
                    // transfer asks Cloud for authority.
                    let source =
                        crate::managed_context::outbound_service::EventConnectionSource::new(
                            runtime.event_connection_registry().clone(),
                            &config,
                            caller_user_id,
                        );
                    crate::managed_context::event_connections::export_selected(
                        &selection.context_selection.event_connections,
                        &source.registry,
                        &source.owner_user_id,
                    )?;
                    let relay = relay_state.try_read().map_err(|_| {
                        crate::managed_context::owner_managed::admission_error(
                            "source kernel relay is temporarily unavailable",
                        )
                    })?;
                    if !relay.connected() {
                        return Err(crate::managed_context::owner_managed::admission_error(
                            "Source kernel is offline or stale",
                        ));
                    }
                    drop(relay);
                    store.prepare_owner_ticket(&config, &runtime, selection)?
                }
                _ => {
                    return Err(crate::managed_context::owner_managed::admission_error(
                        "select exactly one of ticket or interactive ownerManaged",
                    ))
                }
            };
            let status = start_managed_context_outbound_operation(
                config,
                relay_state,
                store,
                provider_account_profiles,
                ticket,
                Some(ManagedContextOutboundCaller {
                    runtime,
                    user_id: caller_user_id.to_string(),
                    session_id: requester.session_id.map(str::to_owned),
                    launch_id: request.launch_id,
                }),
                request.interactive,
            )?;
            Ok(LocalDaemonResponse::ManagedContextTransferStarted { status })
        }
        LocalDaemonRequest::GetManagedContextTransferStatus(request) => {
            store.authorize_status_owner(&config, &request.context_id)?;
            let status =
                store
                    .get(&request.context_id)
                    .ok_or_else(|| DaemonError::ManagedContext {
                        code: "managed_context_transfer_not_found",
                        operation: "get managed context transfer",
                        message: format!(
                            "managed-context transfer `{}` was not found",
                            request.context_id
                        ),
                        retryable: false,
                    })?;
            Ok(LocalDaemonResponse::ManagedContextTransferStatus { status })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "managed context outbound control",
            message: "unsupported request".to_string(),
        }),
    }
}

fn authorize_source_user(config: &DaemonConfig, caller_user_id: &str) -> Result<(), DaemonError> {
    let cloud_user_id = config
        .cloud_relay
        .as_ref()
        .map(|profile| profile.user_id.as_str())
        .ok_or_else(|| DaemonError::ManagedContext {
            code: "managed_context_source_unavailable",
            operation: "authorize managed context transfer",
            message: "source kernel is not connected to Chariox Cloud".to_string(),
            retryable: true,
        })?;
    if caller_user_id != crate::session::DEFAULT_LOCAL_USER_ID && caller_user_id != cloud_user_id {
        return Err(DaemonError::ManagedContext {
            code: "unauthorized",
            operation: "authorize managed context transfer",
            message: "managed-context transfer belongs to another Cloud user".to_string(),
            retryable: false,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistedCloudRelayProfile;

    #[test]
    fn source_transfer_allows_only_the_local_owner_or_cloud_owner() {
        let mut config = DaemonConfig::for_tests();
        assert!(authorize_source_user(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err());
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });

        authorize_source_user(&config, crate::session::DEFAULT_LOCAL_USER_ID)
            .expect("local kernel owner");
        authorize_source_user(&config, "cloud-user-1").expect("Cloud owner");
        assert!(authorize_source_user(&config, "cloud-user-2").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn owner_copy_refuses_an_unusable_event_connection_before_any_operation_starts() {
        use crate::managed_context::outbound_service::{
            ManagedContextOutboundOperationStore, ManagedContextTransferTarget,
        };
        use crate::managed_context::owner_managed::{
            OwnerManagedContextSelection, OwnerManagedDevelopmentSelection,
            OwnerManagedKernelSelection, OwnerManagedTransfer,
        };
        use crate::managed_context::package::ManagedContextEventConnectionSelection;
        crate::test_support::isolated_env_test!();
        let _lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owner-event-preflight-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config_path = root.join("config.toml");
        config.user_config.history.operational.path =
            Some(root.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config = config.with_session_history_root(root.join("sessions"));
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account".into(),
            user_id: "owner".into(),
            realm_id: "realm".into(),
            machine_id: Some(config.host_machine_id.clone()),
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("synthetic-test-enrollment".into()),
            kernel_public_key_thumbprint: Some(
                crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
            ),
            ..Default::default()
        });
        let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let runtime = router.runtime_state();
        let store = ManagedContextOutboundOperationStore::open(root.join("outbound")).unwrap();
        let start = |connection_id: &str| {
            let selection = OwnerManagedTransfer {
                target: ManagedContextTransferTarget {
                    relay_realm_id: "realm".into(),
                    machine_id: "target-machine".into(),
                    kernel_id: "target-kernel".into(),
                    relay_public_key: config.relay_public_key.clone(),
                    key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config.relay_public_key,
                    ),
                },
                context_selection: OwnerManagedContextSelection {
                    event_connections: ManagedContextEventConnectionSelection::Selected {
                        connection_ids: vec![connection_id.to_string()],
                    },
                    kernel_context: OwnerManagedKernelSelection::Empty,
                    development_setup: OwnerManagedDevelopmentSelection::Empty,
                },
            };
            execute_managed_context_outbound_request(
                config.clone(),
                Arc::new(RwLock::new(
                    crate::transport::relay_client::RelayClientState::default(),
                )),
                store.clone(),
                runtime.provider_account_profile_registry().clone(),
                runtime.clone(),
                OutboundRequester {
                    user_id: "owner",
                    session_id: None,
                },
                LocalDaemonRequest::StartManagedContextTransfer(Box::new(
                    crate::local::StartManagedContextTransferRequest {
                        ticket: None,
                        owner_managed: Some(selection),
                        interactive: true,
                        launch_id: None,
                    },
                )),
            )
            .unwrap_err()
        };
        let code = |error: DaemonError| match error {
            DaemonError::ManagedContext { code, .. } => code,
            other => panic!("expected a managed-context refusal, got {other:?}"),
        };
        assert_eq!(code(start("connection-a")), "event_connection_not_found");
        runtime
            .event_connection_registry()
            .upsert(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "event-owner-secret-a",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.github".to_string(),
                    connection_id: "connection-a".to_string(),
                    status: crate::local::EventConnectionStatus::Pending,
                    metadata: serde_json::json!({}),
                    expires_at_ms: None,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
        assert_eq!(code(start("connection-a")), "event_connection_not_ready");
        std::fs::remove_dir_all(root).unwrap();
    }
}
