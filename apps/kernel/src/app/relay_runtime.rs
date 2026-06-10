use std::path::PathBuf;

use arroba_relay::protocol::{DaemonRegistration, RelayProviderAccountSummary};
use tokio::runtime::{Handle, Runtime, RuntimeFlavor};

use crate::app::DaemonApp;
use crate::error::DaemonError;

impl DaemonApp {
    pub fn relay_registration(&mut self) -> DaemonRegistration {
        let available_providers = self.providers.registry().registered_adapter_keys();
        let provider_accounts = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home_dir| crate::slice_provider_auth::inspect_home_provider_auth(&home_dir))
            .unwrap_or_default()
            .into_iter()
            .map(|account| RelayProviderAccountSummary {
                provider: account.provider,
                state: serde_json::to_value(account.state)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_else(|| "unknown".to_string()),
                auth_type: account.auth_type,
                account_id: account.account_id,
                email: account.email,
                organization_id: account.organization_id,
                organization_name: account.organization_name,
                subscription_type: account.subscription_type,
                alias: account.alias,
            })
            .collect();
        DaemonRegistration {
            auth_token: self.config.relay_token.clone().unwrap_or_default(),
            daemon_id: self.config.daemon_id.clone(),
            machine_id: self.config.host_machine_id.clone(),
            machine_alias: self.config.host_machine_alias.clone(),
            os_name: Some(self.config.os_name.clone()),
            kernel_started_at_ms: self.started_at_ms,
            daemon_alias: self.config.daemon_alias.clone(),
            kernel_alias: self.config.daemon_alias.clone(),
            public_key: self.config.relay_public_key.clone(),
            capabilities: vec![
                "kernel_websocket".to_string(),
                "relay_request_proxy".to_string(),
                "relay_peer_transport".to_string(),
                "execution_lease_management".to_string(),
            ],
            available_providers,
            provider_accounts,
            accepting_remote_leases: self.config.accept_remote_leases,
            leased_agent_count: self.leased_agents.len() as u32,
            local_session_count: self.sessions().list_sessions().len() as u32,
        }
    }

    pub(crate) fn block_on_relay_future<F, T>(&self, future: F) -> Result<T, DaemonError>
    where
        F: std::future::Future<Output = Result<T, DaemonError>>,
    {
        if let Ok(handle) = Handle::try_current() {
            match handle.runtime_flavor() {
                RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(future))
                }
                RuntimeFlavor::CurrentThread => Err(DaemonError::LocalTransport {
                    operation: "block relay future",
                    message: "cannot block on a relay future from a current-thread tokio runtime"
                        .to_string(),
                }),
                _ => Err(DaemonError::LocalTransport {
                    operation: "block relay future",
                    message: "unsupported tokio runtime flavor for blocking relay future"
                        .to_string(),
                }),
            }
        } else {
            Runtime::new()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "create relay runtime",
                    message: error.to_string(),
                })?
                .block_on(future)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_relay_future_does_not_panic_on_current_thread_runtime() {
        let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("test daemon app should bootstrap");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        let result = runtime
            .block_on(async { app.block_on_relay_future(async { Ok::<_, DaemonError>(()) }) });

        assert!(matches!(
            result,
            Err(DaemonError::LocalTransport {
                operation: "block relay future",
                ..
            })
        ));
    }
}
