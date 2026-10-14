//! MP-08/MP-11: same-machine carrier for the paired web terminal.
//!
//! A relay-authenticated browser mints a short single-use grant over its
//! existing encrypted relay lane. It then upgrades a literal loopback WebSocket
//! that admits only the paired Cloud Origin, proves possession of the grant and
//! of the browser key bound into its relay identity, and from then on speaks the
//! same encrypted relay client frames. Requests and subscriptions reuse the
//! relay dispatch, command cache and event log, so the carrier adds no
//! authority. The kernel's CLI listener keeps refusing every browser Origin.

use std::collections::BTreeMap;

use base64::Engine;
use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::RelayCallerIdentity;
use rand::RngCore;
use tokio::net::TcpListener;

use super::request_errors::relay_error;
use super::*;

mod admission;
mod leases;
mod session;
#[cfg(test)]
mod tests;

pub(crate) const LOCAL_BROWSER_PATH: &str = "/v1/browser";
const LOCAL_BROWSER_PORT_ENV: &str = "CHARIOX_KERNEL_BROWSER_PORT";
const GRANT_TTL_MS: u64 = 30_000;
/// Cloud stamps identity expiry on its clock and this kernel reads it on the
/// user's clock. On a machine behind Cloud (VMs, WSL, no NTP) a fresh 30-second
/// identity looks longer; the allowance keeps it short without extending any
/// lease past the identity's expiry.
const CLOCK_SKEW_ALLOWANCE_MS: u64 = 10_000;
const MAX_OUTSTANDING_GRANTS: usize = 64;
const AUTHORITY_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// What a direct session was admitted under. Any change closes the session.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalBrowserAuthority {
    realm_id: String,
    account_id: String,
    user_id: String,
    origin: String,
    relay_public_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalDirectClient {
    Browser,
    Terminal,
}

#[derive(Debug, Clone)]
struct LocalBrowserGrant {
    lease_id: String,
    client: LocalDirectClient,
    identity: RelayCallerIdentity,
    thumbprint: String,
    authority: LocalBrowserAuthority,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct LocalBrowserEndpoint {
    port: u16,
    epoch: String,
}

pub(crate) struct LocalBrowserDirect {
    router: Arc<CommandRouter>,
    command_sequence: Arc<AtomicU64>,
    event_runtime: Arc<RelayEventRuntime>,
    command_result_cache: RelayCommandResultCache,
    shutdown: watch::Receiver<bool>,
    port: Option<u16>,
    endpoint: Mutex<Option<LocalBrowserEndpoint>>,
    bound: mpsc::UnboundedSender<(TcpListener, LocalBrowserEndpoint)>,
    grants: std::sync::Mutex<BTreeMap<String, LocalBrowserGrant>>,
    leases: std::sync::Mutex<BTreeMap<String, leases::LocalBrowserLease>>,
    authority: watch::Sender<Option<LocalBrowserAuthority>>,
}

impl LocalBrowserDirect {
    pub(super) fn new(
        router: Arc<CommandRouter>,
        command_sequence: Arc<AtomicU64>,
        event_runtime: Arc<RelayEventRuntime>,
        command_result_cache: RelayCommandResultCache,
        shutdown: watch::Receiver<bool>,
    ) -> Arc<Self> {
        Self::with_port(
            router,
            command_sequence,
            event_runtime,
            command_result_cache,
            shutdown,
            configured_port(std::env::var(LOCAL_BROWSER_PORT_ENV).ok().as_deref()),
        )
    }

    fn with_port(
        router: Arc<CommandRouter>,
        command_sequence: Arc<AtomicU64>,
        event_runtime: Arc<RelayEventRuntime>,
        command_result_cache: RelayCommandResultCache,
        shutdown: watch::Receiver<bool>,
        port: Option<u16>,
    ) -> Arc<Self> {
        let authority = current_authority(&router.relay_config_snapshot());
        let (bound, bound_rx) = mpsc::unbounded_channel();
        let direct = Arc::new(Self {
            router,
            command_sequence,
            event_runtime,
            command_result_cache,
            shutdown,
            port,
            endpoint: Mutex::new(None),
            bound,
            grants: std::sync::Mutex::new(BTreeMap::new()),
            leases: std::sync::Mutex::new(BTreeMap::new()),
            authority: watch::channel(authority).0,
        });
        // Spawned here rather than on bind: the accept loop dispatches requests
        // that can issue grants, so it must not be part of `ensure_endpoint`.
        tokio::spawn(admission::run_when_bound(Arc::clone(&direct), bound_rx));
        direct
    }

    /// Issues a grant for the relay-verified browser identity. The caller has
    /// already proven that the request was encrypted with the identity's key.
    pub(super) async fn issue_grant(
        self: &Arc<Self>,
        identity: &RelayCallerIdentity,
    ) -> Result<serde_json::Value, RelayError> {
        self.issue_client_grant(identity, LocalDirectClient::Browser)
            .await
    }

    pub(super) async fn issue_terminal_grant(
        self: &Arc<Self>,
        identity: &RelayCallerIdentity,
    ) -> Result<serde_json::Value, RelayError> {
        self.issue_client_grant(identity, LocalDirectClient::Terminal)
            .await
    }

    async fn issue_client_grant(
        self: &Arc<Self>,
        identity: &RelayCallerIdentity,
        client: LocalDirectClient,
    ) -> Result<serde_json::Value, RelayError> {
        let authority = self
            .refresh_authority()
            .ok_or_else(|| unavailable("this kernel is not paired with a Cloud origin"))?;
        let now_ms = crate::session::unix_epoch_ms();
        let thumbprint = identity.public_key_thumbprint.clone().unwrap_or_default();
        if identity.subject_kind != RelaySubjectKind::Client
            || identity.user_id.is_none()
            || identity.realm_id != authority.realm_id
            || thumbprint.is_empty()
        {
            return Err(relay_error(
                "unauthorized",
                "local browser connect requires a live paired terminal identity",
                false,
            ));
        }
        let expires_at_ms = client_identity_deadline(identity, now_ms, client).ok_or_else(|| {
            relay_error(
                "unauthorized",
                "local browser connect requires a live 30-second relay identity; check that the system clock is synchronized",
                false,
            )
        })?;
        let paired_origin = authority.origin.clone();
        let endpoint = self.ensure_endpoint().await?;
        let grant = random_token();
        {
            let mut grants = self.grants.lock().expect("local browser grants poisoned");
            grants.retain(|_, grant| grant.expires_at_ms > now_ms);
            while grants.len() >= MAX_OUTSTANDING_GRANTS {
                let oldest = grants
                    .iter()
                    .min_by_key(|(_, grant)| grant.expires_at_ms)
                    .map(|(id, _)| id.clone());
                match oldest {
                    Some(id) => grants.remove(&id),
                    None => break,
                };
            }
            grants.insert(
                grant.clone(),
                LocalBrowserGrant {
                    lease_id: grant.clone(),
                    client,
                    identity: identity.clone(),
                    thumbprint,
                    authority,
                    expires_at_ms,
                },
            );
        }
        let mut issued = serde_json::json!({
            "endpoint": format!("ws://127.0.0.1:{}{LOCAL_BROWSER_PATH}", endpoint.port),
            "grant": grant,
            "kernel_id": self.router.relay_daemon_id(),
            "endpoint_epoch": endpoint.epoch,
            "expires_at_ms": expires_at_ms,
        });
        let name = match client {
            LocalDirectClient::Browser => "LocalBrowserConnectIssued",
            LocalDirectClient::Terminal => {
                issued["paired_origin"] = serde_json::json!(paired_origin);
                "LocalTerminalConnectIssued"
            }
        };
        Ok(serde_json::json!({name: issued}))
    }

    /// Single-use redemption: a grant is removed before it is checked.
    fn redeem_grant(
        &self,
        grant: &str,
        thumbprint: &str,
    ) -> Result<LocalBrowserGrant, &'static str> {
        let redeemed = self
            .grants
            .lock()
            .expect("local browser grants poisoned")
            .remove(grant)
            .ok_or("local browser grant is unknown or already used")?;
        if redeemed.expires_at_ms <= crate::session::unix_epoch_ms() {
            return Err("local browser grant has expired");
        }
        if redeemed.thumbprint != thumbprint {
            return Err("local browser key does not match its grant");
        }
        if self.authority.borrow().as_ref() != Some(&redeemed.authority) {
            return Err("local browser authority changed after the grant was issued");
        }
        Ok(redeemed)
    }

    async fn ensure_endpoint(self: &Arc<Self>) -> Result<LocalBrowserEndpoint, RelayError> {
        let mut endpoint = self.endpoint.lock().await;
        if let Some(endpoint) = endpoint.as_ref() {
            return Ok(endpoint.clone());
        }
        let port = self
            .port
            .ok_or_else(|| unavailable("local browser connections are disabled on this kernel"))?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                crate::logging::warn_with_fields(
                    "daemon.local_browser",
                    "local browser endpoint unavailable",
                    serde_json::json!({"port": port, "error": error.to_string()}),
                );
                unavailable("local browser endpoint port is unavailable")
            })?;
        let bound = LocalBrowserEndpoint {
            port: listener
                .local_addr()
                .map_err(|_| unavailable("local browser endpoint port is unavailable"))?
                .port(),
            epoch: random_token(),
        };
        crate::logging::info_with_fields(
            "daemon.local_browser",
            "local browser endpoint listening",
            serde_json::json!({"port": bound.port}),
        );
        self.bound
            .send((listener, bound.clone()))
            .map_err(|_| unavailable("local browser endpoint is shutting down"))?;
        *endpoint = Some(bound.clone());
        Ok(bound)
    }

    /// Revocation: logout, re-pairing to another account or realm, Cloud
    /// origin change, or kernel key rotation retires every direct session.
    async fn watch_authority(&self) {
        let mut shutdown = self.shutdown.clone();
        loop {
            tokio::select! {
                _ = shutdown.changed() => return,
                _ = sleep(AUTHORITY_POLL_INTERVAL) => {}
            }
            if *shutdown.borrow() {
                return;
            }
            self.refresh_authority();
        }
    }

    /// Publishes the current pairing authority; a change retires every session.
    fn refresh_authority(&self) -> Option<LocalBrowserAuthority> {
        let next = current_authority(&self.router.relay_config_snapshot());
        self.authority.send_if_modified(|current| {
            if *current == next {
                return false;
            }
            *current = next.clone();
            true
        });
        next
    }
}

fn current_authority(config: &crate::config::DaemonConfig) -> Option<LocalBrowserAuthority> {
    let profile = config.cloud_relay.as_ref()?;
    Some(LocalBrowserAuthority {
        realm_id: profile.realm_id.clone(),
        account_id: profile.account_id.clone(),
        user_id: profile.user_id.clone(),
        origin: paired_cloud_origin(&profile.api_url)?,
        relay_public_key: config.relay_public_key.clone(),
    })
}

/// The exact web Origin of the paired Cloud: HTTPS, or HTTP on loopback for
/// local development. Anything else is not a browser origin we admit.
fn paired_cloud_origin(api_url: &str) -> Option<String> {
    let url = url::Url::parse(api_url).ok()?;
    let loopback = matches!(
        url.host(),
        Some(url::Host::Ipv4(ip)) if ip.is_loopback()
    ) || url.host_str() == Some("localhost");
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return None;
    }
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Ephemeral by default so every kernel on a machine gets its own endpoint;
/// the grant carries the bound URL.
fn configured_port(value: Option<&str>) -> Option<u16> {
    match value.map(str::trim) {
        None | Some("") => Some(0),
        Some("off") => None,
        Some(value) => value.parse::<u16>().ok(),
    }
}

/// The lease a short, live relay identity earns at `now_ms`: at most
/// `GRANT_TTL_MS` and never past the identity's expiry. `None` when the
/// identity is expired or long-lived even allowing for clock skew.
fn short_identity_deadline(identity: &RelayCallerIdentity, now_ms: u64) -> Option<u64> {
    let short =
        identity.expires_at_ms <= now_ms.saturating_add(GRANT_TTL_MS + CLOCK_SKEW_ALLOWANCE_MS);
    (identity.expires_at_ms > now_ms && short)
        .then(|| identity.expires_at_ms.min(now_ms + GRANT_TTL_MS))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unavailable(message: &str) -> RelayError {
    relay_error("local_browser_unavailable", message, false)
}

// MP-08/MP-11 protocol 473: an explicit terminal lease is short even when
// the CLI's bound relay identity is long-lived. Every extension re-enters the
// authenticated relay dispatcher; the direct session cannot renew itself.
fn client_identity_deadline(
    identity: &RelayCallerIdentity,
    now: u64,
    client: LocalDirectClient,
) -> Option<u64> {
    match client {
        LocalDirectClient::Browser => short_identity_deadline(identity, now),
        LocalDirectClient::Terminal => (identity.expires_at_ms > now)
            .then(|| identity.expires_at_ms.min(now.saturating_add(GRANT_TTL_MS))),
    }
}
