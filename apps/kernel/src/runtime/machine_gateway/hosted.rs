//! The hosted gateway's row in the Cloud clients list.
//!
//! The kernel registers the gateway before its first hosted link, and again after
//! the owner revoked it there. Both happen only under an agent access grant the
//! owner approved (grants die with the kernel and end with a revoke), so
//! registering is what brings a revoked gateway back.
use tokio::sync::Mutex;

use super::error::GatewayError;
use crate::config::PersistedCloudRelayProfile;
use crate::runtime::cloud_api_client::register_cloud_gateway_client;

pub(super) struct HostedClient {
    registered: Mutex<bool>,
}

impl HostedClient {
    pub(super) fn new() -> Self {
        Self {
            registered: Mutex::new(false),
        }
    }

    pub(super) async fn ensure_registered(
        &self,
        profile: &PersistedCloudRelayProfile,
        client_id: &str,
        thumbprint: &str,
        machine: &str,
    ) -> Result<(), GatewayError> {
        let mut registered = self.registered.lock().await;
        if *registered {
            return Ok(());
        }
        let local = profile
            .machine_alias
            .as_deref()
            .filter(|alias| !alias.trim().is_empty())
            .or(profile.machine_id.as_deref())
            .unwrap_or("this machine");
        register_cloud_gateway_client(
            profile,
            client_id,
            thumbprint,
            &format!("Agent gateway of {local}"),
        )
        .await
        .map_err(|error| GatewayError::Unreachable {
            name: machine.into(),
            detail: format!("could not register the agent gateway with Cloud: {error}"),
        })?;
        *registered = true;
        Ok(())
    }

    /// The owner revoked the row: the next registration happens under a new grant.
    pub(super) async fn forget(&self) {
        *self.registered.lock().await = false;
    }
}

/// Cloud refused a gateway token because its client row was revoked.
pub(super) fn is_revoked(error: &crate::error::DaemonError) -> bool {
    error.to_string().contains("cloud_api_code=identity_revoked")
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

    #[tokio::test]
    async fn registers_once_and_again_only_after_the_owner_revoked_it() {
        let (url, seen) = cloud();
        let profile = enrolled(url);
        let hosted = HostedClient::new();
        for _ in 0..3 {
            hosted
                .ensure_registered(&profile, "agent-gateway", &"d".repeat(64), "b3")
                .await
                .unwrap();
        }
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        hosted.forget().await;
        hosted
            .ensure_registered(&profile, "agent-gateway", &"d".repeat(64), "b3")
            .await
            .unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_registration_cloud_refuses_is_unreachable_and_retried_next_time() {
        let hosted = HostedClient::new();
        let error = hosted
            .ensure_registered(&enrolled("http://127.0.0.1:9".into()), "agent-gateway", &"d".repeat(64), "b3")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "machine_unreachable");
        assert!(error.message().contains("agent gateway"));
        assert!(!*hosted.registered.lock().await);
    }

    #[test]
    fn only_cloud_identity_revoked_counts_as_a_revoked_gateway() {
        let revoked = crate::error::DaemonError::RelayTransport {
            operation: "issue Cloud terminal client token",
            code: "cloud_request_failed".into(),
            retryable: false,
            message: "cloud relay request failed with 403: cloud_api_code=identity_revoked: gone".into(),
        };
        let denied = crate::error::DaemonError::RelayTransport {
            operation: "issue Cloud terminal client token",
            code: "cloud_request_failed".into(),
            retryable: false,
            message: "cloud relay request failed with 403: cloud_api_code=authorization_denied".into(),
        };
        assert!(is_revoked(&revoked));
        assert!(!is_revoked(&denied));
    }
}
