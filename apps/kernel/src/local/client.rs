use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::command::KernelCommandSource;
use crate::runtime::router::{CommandRouter, INTERACTIVE_COMMAND_QUEUE_LIMIT};
use crate::session::unix_epoch_ms;

use super::{LocalDaemonRequest, LocalDaemonResponse};

/// In-process local request client for tests and smoke harnesses.
///
/// This keeps local request callers on the same `CommandRouter` boundary as IPC
/// and kernel transport without exposing `DaemonApp::handle_local_request`.
pub struct LocalDaemonClient {
    router: CommandRouter,
    runtime: Runtime,
    command_sequence: AtomicU64,
}

impl LocalDaemonClient {
    pub fn new(app: DaemonApp) -> Result<Self, DaemonError> {
        let provider_runtime_lanes = app.provider_run_operation_lanes();
        let app = Arc::new(Mutex::new(app));
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create local daemon client runtime",
                message: error.to_string(),
            })?;
        let router = runtime.block_on(async {
            CommandRouter::with_interactive_capacity_and_provider_lanes(
                Arc::clone(&app),
                INTERACTIVE_COMMAND_QUEUE_LIMIT,
                provider_runtime_lanes,
            )
        });

        Ok(Self {
            router,
            runtime,
            command_sequence: AtomicU64::new(1),
        })
    }

    pub fn send(&self, request: LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        let sequence = self.command_sequence.fetch_add(1, Ordering::Relaxed);
        let command_id = format!("local-client-{}-{sequence}", unix_epoch_ms());
        self.runtime.block_on(async {
            let caller = self
                .router
                .local_command_caller(KernelCommandSource::LocalIpc)
                .await;
            let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
                command_id,
                KernelCommandSource::LocalIpc,
                caller,
                None,
                None,
                &request,
            );
            self.router.dispatch(command, request).await
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::PersistedCloudRelayProfile;
    use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    use super::LocalDaemonClient;

    #[test]
    fn local_client_uses_linked_cloud_user_for_session_creation() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "miguel@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-cloud".to_string(),
            account_slug: "miguel".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "ws://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: Some("local-cli".to_string()),
            machine_id: Some("machine-1".to_string()),
            machine_alias: Some("macbook".to_string()),
            cloud_session_token: Some("session-token".to_string()),
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let client = LocalDaemonClient::new(app).expect("local daemon client should start");

        let response = client
            .send(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-client", "."),
            ))
            .expect("session create should succeed");
        let session = match response {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected response: {other:?}"),
        };

        assert_eq!(session.owner_user_id(), "user-cloud");
        assert!(session.has_member("user-cloud"));
    }
}
