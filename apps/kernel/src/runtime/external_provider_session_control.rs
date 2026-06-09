use crate::error::DaemonError;
use crate::local::{
    ExternalProviderSessionPage, LocalDaemonRequest, LocalDaemonResponse,
    WatchExternalProviderSessionStatusRequest,
};
use crate::session::unix_epoch_ms;

pub(crate) async fn execute_external_provider_session_request(
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(_request) => {
            Ok(LocalDaemonResponse::ExternalProviderSessionsListed {
                page: empty_external_provider_session_page(),
            })
        }
        LocalDaemonRequest::RefreshExternalProviderSessions(_request) => {
            Ok(LocalDaemonResponse::ExternalProviderSessionsRefreshed {
                page: empty_external_provider_session_page(),
            })
        }
        LocalDaemonRequest::WatchExternalProviderSessionStatus(request) => {
            Ok(watch_status_response(request))
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

fn empty_external_provider_session_page() -> ExternalProviderSessionPage {
    ExternalProviderSessionPage {
        sessions: Vec::new(),
        next_cursor: None,
        has_more: false,
        generated_at_ms: unix_epoch_ms(),
    }
}

fn watch_status_response(
    request: WatchExternalProviderSessionStatusRequest,
) -> LocalDaemonResponse {
    LocalDaemonResponse::ExternalProviderSessionWatchStatus {
        external_session_id: request.external_session_id,
        status: "unavailable".to_string(),
    }
}
