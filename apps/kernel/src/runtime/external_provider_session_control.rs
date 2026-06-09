use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    ListExternalProviderSessionsRequest, LocalDaemonRequest, LocalDaemonResponse,
    WatchExternalProviderSessionStatusRequest,
};

pub(crate) async fn execute_external_provider_session_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let store = {
        let app = app.lock().await;
        app.external_provider_session_index_store()
    };
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(request) => {
            Ok(LocalDaemonResponse::ExternalProviderSessionsListed {
                page: store.list(&request),
            })
        }
        LocalDaemonRequest::RefreshExternalProviderSessions(request) => {
            let provider = request.provider.clone();
            let discovered = crate::app::discover_external_provider_sessions(provider.as_deref());
            if let Some(provider) = provider.as_deref() {
                store.replace_provider_sessions(provider, discovered);
            } else {
                for provider in ["codex", "claude", "opencode"] {
                    let provider_sessions = discovered
                        .iter()
                        .filter(|session| session.provider == provider)
                        .cloned()
                        .collect::<Vec<_>>();
                    store.replace_provider_sessions(provider, provider_sessions);
                }
            }
            let list_request = ListExternalProviderSessionsRequest {
                provider: request.provider,
                cursor: None,
                limit: None,
            };
            Ok(LocalDaemonResponse::ExternalProviderSessionsRefreshed {
                page: store.list(&list_request),
            })
        }
        LocalDaemonRequest::WatchExternalProviderSessionStatus(request) => {
            Ok(watch_status_response(&store, request))
        }
        LocalDaemonRequest::ImportExternalProviderSession(_)
        | LocalDaemonRequest::ImportExternalProviderAgent(_) => Err(DaemonError::LocalTransport {
            operation: "import external provider session",
            message: "external provider session index is not initialized".to_string(),
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "external provider session request",
            message: "unsupported external provider session request".to_string(),
        }),
    }
}

fn watch_status_response(
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: WatchExternalProviderSessionStatusRequest,
) -> LocalDaemonResponse {
    let status = store
        .get(&request.external_session_id)
        .map(|session| {
            if session.already_imported {
                "imported".to_string()
            } else {
                session
                    .running_state
                    .unwrap_or_else(|| "available".to_string())
            }
        })
        .unwrap_or_else(|| "unavailable".to_string());
    LocalDaemonResponse::ExternalProviderSessionWatchStatus {
        external_session_id: request.external_session_id,
        status,
    }
}
