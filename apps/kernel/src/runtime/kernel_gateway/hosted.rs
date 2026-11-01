//! The hosted gateway's row in the Cloud clients list.
//!
//! Every access grant the user approves registers the gateway before its first
//! hosted link, which lists it and brings it back after the user revoked it there.
//! When Cloud refuses a gateway token because the row was revoked, the grants that
//! were live end (the user took the agents' machine access away), and the next
//! grant registers again.
use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use super::error::GatewayError;
use crate::config::PersistedCloudRelayProfile;
use crate::runtime::cloud_api_client::register_cloud_gateway_client;

/// Grants are few and short-lived; the set is dropped when it grows past this.
const REMEMBERED_GRANTS: usize = 256;

#[derive(Default)]
struct State {
    /// The owner revoked the row; stays so until the agent access grants have ended.
    revoked: bool,
    registered_for: HashSet<String>,
}

#[derive(Clone, Default)]
pub(super) struct HostedClient {
    state: Arc<Mutex<State>>,
}

impl HostedClient {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) async fn ensure_registered(
        &self,
        profile: &PersistedCloudRelayProfile,
        client_id: &str,
        thumbprint: &str,
        grant_id: &str,
        kernel_alias: Option<&str>,
        target: &str,
    ) -> Result<(), GatewayError> {
        let mut state = self.state.lock().await;
        if state.revoked {
            return Err(GatewayError::GatewayRevoked);
        }
        if state.registered_for.contains(grant_id) {
            return Ok(());
        }
        register_cloud_gateway_client(
            profile,
            client_id,
            thumbprint,
            &client_name(profile, kernel_alias),
        )
        .await
        .map_err(|error| GatewayError::Unreachable {
            name: target.into(),
            detail: super::error::plain_cause(
                "could not register the agent gateway with Cloud",
                &error,
            ),
        })?;
        if state.registered_for.len() >= REMEMBERED_GRANTS {
            state.registered_for.clear();
        }
        state.registered_for.insert(grant_id.to_string());
        Ok(())
    }

    /// The owner revoked the row. True for the caller that found it first.
    pub(super) async fn revoked(&self) -> bool {
        let mut state = self.state.lock().await;
        state.registered_for.clear();
        !std::mem::replace(&mut state.revoked, true)
    }

    /// The grants that were live at the revoke have ended: the next grant registers afresh.
    pub(super) async fn reopen(&self) {
        self.state.lock().await.revoked = false;
    }
}

/// The row the owner sees in the clients list. A machine can run several of the user's
/// kernels, each with its own gateway, so the kernel's alias tells the rows apart.
fn client_name(profile: &PersistedCloudRelayProfile, kernel_alias: Option<&str>) -> String {
    let machine = profile
        .machine_alias
        .as_deref()
        .filter(|alias| !alias.trim().is_empty())
        .or(profile.machine_id.as_deref())
        .unwrap_or("this machine");
    match kernel_alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
    {
        Some(alias) if alias != machine => format!("Agent gateway of {machine}/{alias}"),
        _ => format!("Agent gateway of {machine}"),
    }
}

/// Cloud refused a gateway token because its client row was revoked.
pub(super) fn is_revoked(error: &crate::error::DaemonError) -> bool {
    error
        .to_string()
        .contains("cloud_api_code=identity_revoked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A Cloud that accepts every gateway registration and counts them.
    fn cloud() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buffer = [0; 8192];
                let _ = stream.read(&mut buffer);
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
            }
        });
        (url, seen)
    }

    fn enrolled(url: String) -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: url,
            kernel_id: Some("kernel-fixture".into()),
            kernel_credential: Some("synthetic-kernel-credential".into()),
            machine_alias: Some("Studio".into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_clients_list_row_names_the_kernel_when_a_machine_runs_several() {
        let profile = enrolled("http://unused".into());
        assert_eq!(client_name(&profile, None), "Agent gateway of Studio");
        assert_eq!(client_name(&profile, Some("  ")), "Agent gateway of Studio");
        assert_eq!(
            client_name(&profile, Some("Studio")),
            "Agent gateway of Studio"
        );
        assert_eq!(
            client_name(&profile, Some("dev")),
            "Agent gateway of Studio/dev"
        );
    }

    async fn register(
        hosted: &HostedClient,
        profile: &PersistedCloudRelayProfile,
        grant: &str,
    ) -> Result<(), GatewayError> {
        hosted
            .ensure_registered(profile, "agent-gateway", &"d".repeat(64), grant, None, "b3")
            .await
    }

    #[tokio::test]
    async fn each_grant_registers_once_so_a_new_grant_brings_a_revoked_gateway_back() {
        let (url, seen) = cloud();
        let profile = enrolled(url);
        let hosted = HostedClient::new();
        for _ in 0..3 {
            register(&hosted, &profile, "grant-1").await.unwrap();
        }
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        register(&hosted, &profile, "grant-2").await.unwrap();
        assert_eq!(
            seen.load(Ordering::SeqCst),
            2,
            "a new grant registers afresh"
        );
    }

    #[tokio::test]
    async fn a_revoked_gateway_asks_cloud_nothing_until_the_grants_have_ended() {
        let (url, seen) = cloud();
        let profile = enrolled(url);
        let hosted = HostedClient::new();
        register(&hosted, &profile, "grant-1").await.unwrap();
        assert!(hosted.revoked().await);
        assert!(!hosted.revoked().await, "only the first finder acts");
        let refused = register(&hosted, &profile, "grant-1").await.unwrap_err();
        assert_eq!(refused.code(), "kernel_unpaired");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        hosted.reopen().await;
        register(&hosted, &profile, "grant-1").await.unwrap();
        assert_eq!(
            seen.load(Ordering::SeqCst),
            2,
            "the revoke forgot the registration"
        );
    }

    #[tokio::test]
    async fn a_registration_cloud_refuses_is_unreachable_and_retried_next_time() {
        let hosted = HostedClient::new();
        let error = register(&hosted, &enrolled("http://127.0.0.1:9".into()), "grant-1")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "kernel_unreachable");
        assert!(error.message().contains("agent gateway"));
        assert!(hosted.state.lock().await.registered_for.is_empty());
    }

    #[test]
    fn only_cloud_identity_revoked_counts_as_a_revoked_gateway() {
        let revoked = crate::error::DaemonError::RelayTransport {
            operation: "issue Cloud terminal client token",
            code: "cloud_request_failed".into(),
            retryable: false,
            message: "cloud relay request failed with 403: cloud_api_code=identity_revoked: gone"
                .into(),
        };
        let denied = crate::error::DaemonError::RelayTransport {
            operation: "issue Cloud terminal client token",
            code: "cloud_request_failed".into(),
            retryable: false,
            message: "cloud relay request failed with 403: cloud_api_code=authorization_denied"
                .into(),
        };
        assert!(is_revoked(&revoked));
        assert!(!is_revoked(&denied));
    }
}
