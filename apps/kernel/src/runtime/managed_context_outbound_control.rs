use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::managed_context::outbound_service::{
    start_managed_context_outbound_operation, ManagedContextOutboundOperationStore,
};
use crate::transport::relay_client::RelayClientState;

pub(crate) fn execute_managed_context_outbound_request(
    config: DaemonConfig,
    relay_state: Arc<RwLock<RelayClientState>>,
    store: ManagedContextOutboundOperationStore,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    authorize_source_user(&config, caller_user_id)?;
    match request {
        LocalDaemonRequest::StartManagedContextTransfer(request) => {
            let status = start_managed_context_outbound_operation(
                config,
                relay_state,
                store,
                request.ticket,
            )?;
            Ok(LocalDaemonResponse::ManagedContextTransferStarted { status })
        }
        LocalDaemonRequest::GetManagedContextTransferStatus(request) => {
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
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });

        authorize_source_user(&config, crate::session::DEFAULT_LOCAL_USER_ID)
            .expect("local kernel owner");
        authorize_source_user(&config, "cloud-user-1").expect("Cloud owner");
        assert!(authorize_source_user(&config, "cloud-user-2").is_err());
    }
}
