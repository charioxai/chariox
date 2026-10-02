use chariox_relay::protocol::{DaemonRegistration, RelayProviderAccountSummary};
use tokio::runtime::{Handle, Runtime, RuntimeFlavor};

use crate::app::DaemonApp;
use crate::error::DaemonError;

impl DaemonApp {
    pub fn relay_registration(&mut self) -> DaemonRegistration {
        let available_providers = self.providers.registry().advertised_provider_ids();
        let provider_accounts = self
            .provider_account_profiles
            .list(crate::session::DEFAULT_LOCAL_USER_ID, None)
            .unwrap_or_default()
            .into_iter()
            .map(|account| RelayProviderAccountSummary {
                provider: account.provider,
                state: serde_json::to_value(account.auth_state)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_else(|| "unknown".to_string()),
                auth_type: None,
                account_id: Some(account.profile_id),
                email: account.identity_summary,
                organization_id: None,
                organization_name: None,
                subscription_type: account.plan,
                alias: Some(account.label),
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
        F: std::future::Future<Output = Result<T, DaemonError>> + Send,
        T: Send,
    {
        if let Ok(handle) = Handle::try_current() {
            match handle.runtime_flavor() {
                RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(future))
                }
                RuntimeFlavor::CurrentThread => std::thread::scope(|scope| {
                    scope
                        .spawn(move || {
                            Runtime::new()
                                .map_err(|error| DaemonError::LocalTransport {
                                    operation: "create relay runtime",
                                    message: error.to_string(),
                                })?
                                .block_on(future)
                        })
                        .join()
                        .map_err(|_| DaemonError::LocalTransport {
                            operation: "block relay future",
                            message: "relay runtime worker thread panicked".to_string(),
                        })?
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
    fn block_on_relay_future_uses_a_scoped_runtime_from_current_thread_runtime() {
        let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("test daemon app should bootstrap");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        let result = runtime
            .block_on(async { app.block_on_relay_future(async { Ok::<_, DaemonError>(()) }) });

        result.expect("scoped relay runtime should complete");
    }
}
