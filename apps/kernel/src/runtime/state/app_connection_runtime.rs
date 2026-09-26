//! Protocol 359: the owner lets an installation act through one of their
//! event generator connections. The router checked the connection with its
//! generator; here the App's signed manifest must declare that generator.
//! Which actions it may take there comes only from that manifest.
use super::KernelRuntimeState;
use crate::durable_state::app_active_release::ActiveReleaseError;
use crate::durable_state::app_connections::ConnectionGrantCommand;
use crate::local::{
    AppConnectionSummary, AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse,
};

impl KernelRuntimeState {
    pub(super) async fn app_connection_request(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let access = self.app_connection_access(&owner, &installation).await?;
        let command = match request {
            LocalDaemonRequest::GrantAppConnection(request) => {
                if !access
                    .iter()
                    .any(|declared| declared.generator == request.generator_id)
                {
                    return Err(AppRequestErrorCode::InvalidRequest);
                }
                Some(ConnectionGrantCommand::Grant {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    generator_id: request.generator_id,
                    connection_id: request.connection_id,
                    now_ms: crate::session::unix_epoch_ms(),
                })
            }
            LocalDaemonRequest::RevokeAppConnection(request) => {
                Some(ConnectionGrantCommand::Revoke {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    connection_id: request.connection_id,
                })
            }
            LocalDaemonRequest::ListAppConnections(_) => None,
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        };
        let store = self.owned.durable_state_store.clone();
        let permit = self.app_control().try_admit()?;
        let (list_owner, list_installation) = (owner.clone(), installation.clone());
        let grants = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(command) = command {
                store.app_connection_grant(command)?;
            }
            store.app_connection_grants(&list_owner, &list_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|code| match code {
            "NOT_FOUND" => AppRequestErrorCode::NotFound,
            "LIMIT_EXCEEDED" => AppRequestErrorCode::LimitExceeded,
            _ => AppRequestErrorCode::StorageUnavailable,
        })?;
        Ok(LocalDaemonResponse::AppConnections {
            installation_id: installation,
            connections: grants
                .into_iter()
                .map(|grant| AppConnectionSummary {
                    actions: access
                        .iter()
                        .find(|declared| declared.generator == grant.generator_id)
                        .map(|declared| declared.actions.clone())
                        .unwrap_or_default(),
                    generator_id: grant.generator_id,
                    connection_id: grant.connection_id,
                    granted_at_ms: grant.granted_at_ms,
                })
                .collect(),
        })
    }

    pub(crate) async fn app_connection_access(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<chariox_app_package::ConnectionAccess>, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.active_app_connection_access(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            ActiveReleaseError::NotActive => AppRequestErrorCode::NotFound,
            ActiveReleaseError::Untrusted | ActiveReleaseError::Invalid => {
                AppRequestErrorCode::Conflict
            }
            ActiveReleaseError::Unavailable | ActiveReleaseError::Storage => {
                AppRequestErrorCode::StorageUnavailable
            }
        })
    }
}
