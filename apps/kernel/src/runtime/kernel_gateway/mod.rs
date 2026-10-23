//! The kernel gateway of the LOCAL kernel (External Agents v2).
//!
//! An external agent holding a local grant names a kernel on a request frame;
//! this kernel authorizes the request locally, then forwards it over the relay as
//! a terminal-class client of the target kernel, which stays the authority for
//! its own state. The agent never sees a relay token, key or Cloud credential.
mod directory;
mod error;
mod events;
mod events_created;
mod hosted;
mod identity;
mod inventory;
mod join;
mod keeper;
pub(crate) mod ledger;
mod link;
mod names;
pub(crate) mod origins;
mod paired;
mod self_hosted;
mod subscription;
mod sudo_watch;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::local::{LocalDaemonRequest, OnBehalfOf};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;

use directory::{is_gateway_ready, Directory};
pub(crate) use error::{is_public_code, GatewayError};
use identity::GatewayIdentity;
use inventory::RelayInventory;
use link::{Link, LinkFailure, LinkParams};
pub(crate) use subscription::{ForwardedSubscription, SubscribeRequest};

/// The default time a forwarded request may take before `kernel_request_timeout`.
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const GATEWAY_CLIENT_ID: &str = "agent-gateway";
const GRANTS_END_AFTER_REVOKE: Duration = Duration::from_secs(2);
pub(super) static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The grant a forwarded request acts under, as the origin kernel knows it.
/// How often a long wait or a watch asks the target whether it still accepts the gateway.
const REVOCATION_PROBE: Duration = Duration::from_secs(4);

/// What the target's popup calls the requesting agent: the provider harness the origin kernel
/// recognised, else the holder's executable name (a version number or `node` says nothing).
pub(crate) fn harness_name(grant: &crate::local::KernelAccessGrant) -> String {
    use crate::local::KernelAccessProviderHarness::{Claude, Codex, Opencode};
    match grant.holder_harness {
        Some(Claude) => "claude".into(),
        Some(Codex) => "codex".into(),
        Some(Opencode) => "opencode".into(),
        None => std::path::Path::new(&grant.holder_executable)
            .file_name()
            .map_or_else(
                || grant.holder_executable.clone(),
                |name| name.to_string_lossy().into(),
            ),
    }
}

pub(crate) struct Origin {
    pub(crate) grant_id: String,
    pub(crate) harness: String,
}

pub(crate) struct KernelGateway {
    directory: Directory,
    config: DaemonConfigProjectionStore,
    runtime_state: KernelRuntimeState,
    identity: std::sync::Mutex<Option<GatewayIdentity>>,
    links: Mutex<HashMap<String, Arc<Link>>>,
    ledger: OnceLock<ledger::GatewayLedger>,
    keeper: keeper::Keeper,
    sudo_watch: sudo_watch::SudoWatch,
    event_parts: events::EventParts,
    hosted: hosted::HostedClient,
    paired: Arc<paired::PairedKernels>,
    /// When each paired kernel last answered, for the self-hosted list.
    last_answer: std::sync::Mutex<HashMap<String, u64>>,
}

impl KernelGateway {
    pub(crate) fn new(
        runtime_state: KernelRuntimeState,
        config: DaemonConfigProjectionStore,
        projection: RemoteRelayInventoryProjectionStore,
    ) -> Self {
        let paired = Arc::new(paired::PairedKernels::new(
            config
                .snapshot()
                .private_runtime_state_root()
                .join("gateway-kernels.json"),
        ));
        let source = Arc::new(RelayInventory {
            config: config.clone(),
            projection,
        });
        Self {
            directory: Directory::new(source),
            config,
            runtime_state,
            identity: std::sync::Mutex::new(None),
            links: Mutex::new(HashMap::new()),
            ledger: OnceLock::new(),
            keeper: keeper::Keeper::default(),
            sudo_watch: sudo_watch::SudoWatch::default(),
            event_parts: events::EventParts::default(),
            hosted: hosted::HostedClient::new(),
            paired,
            last_answer: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// A relay request reached this kernel with `client_public_key`; false when that
    /// key is a gateway the owner revoked here.
    pub(crate) fn admit_origin(
        &self,
        client_public_key: &str,
        origin: Option<origins::Named<'_>>,
    ) -> Result<bool, crate::error::DaemonError> {
        origins::admit(
            &origins::path_in(&self.config.snapshot().private_runtime_state_root()),
            client_public_key,
            origin,
            crate::session::unix_epoch_ms(),
        )
    }

    /// The durable ledger a target keeps for requests forwarded to it.
    pub(crate) fn ledger(&self) -> &ledger::GatewayLedger {
        self.ledger.get_or_init(|| {
            ledger::GatewayLedger::open(
                self.config
                    .snapshot()
                    .private_runtime_state_root()
                    .join("gateway-ledger.jsonl"),
            )
        })
    }

    /// Forwards `request` to `kernel` and returns the target's response value.
    pub(crate) async fn forward(
        &self,
        target: &str,
        origin: &Origin,
        command_id: &str,
        request: &LocalDaemonRequest,
        deadline: Duration,
    ) -> Result<Value, GatewayError> {
        self.send(target, origin, command_id, request, deadline, true)
            .await
    }

    /// Tells `target` that the grant `origin` acted under has ended. The one request
    /// that is sent after its grant is gone, so it checks nothing about the grant.
    pub(crate) async fn forward_grant_ended(
        &self,
        target: &str,
        origin: &Origin,
        command_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<Value, GatewayError> {
        self.send(target, origin, command_id, request, REQUEST_DEADLINE, false)
            .await
    }

    async fn send(
        &self,
        target: &str,
        origin: &Origin,
        command_id: &str,
        request: &LocalDaemonRequest,
        deadline: Duration,
        grant_live: bool,
    ) -> Result<Value, GatewayError> {
        if grant_live {
            self.ensure_access_live(origin, target)?;
        }
        let kernel = self.resolve(target).await?;
        if grant_live {
            self.register_for(&origin.grant_id, target).await?;
        }
        let gateway_ready = is_gateway_ready(&kernel);
        if let Some(detail) = directory::incompatibility(&kernel, request) {
            return Err(GatewayError::Incompatible {
                name: target.into(),
                detail,
            });
        }
        let mut envelope = serde_json::json!({"command_id": command_id, "request": request});
        if gateway_ready {
            let own = self.config.snapshot();
            envelope["on_behalf_of"] = serde_json::to_value(OnBehalfOf {
                origin_name: Some(names::own_name(&own)),
                origin_kernel: own.daemon_id,
                grant_id: origin.grant_id.clone(),
                harness: origin.harness.clone(),
            })
            .expect("on_behalf_of serializes");
        }
        let plaintext = serde_json::to_vec(&envelope).expect("envelope serializes");
        let link = self.link_for(target, &kernel).await?;
        if grant_live {
            self.ensure_access_live(origin, target)?;
        }
        let request_id = format!(
            "gw-{}-{}",
            crate::session::unix_epoch_ms(),
            REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        match self
            .request_keeping_token_fresh(&link, &kernel, target, request_id, &plaintext, deadline)
            .await
        {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|error| GatewayError::Unreachable {
                    name: target.into(),
                    detail: format!("unreadable answer: {error}"),
                })
            }
            Err(failure) => {
                if matches!(&failure, LinkFailure::Relay(error) if error.message.starts_with("invalid relay request payload"))
                {
                    self.links.lock().await.remove(&kernel.kernel_id);
                }
                let error = map_failure(target, failure);
                if matches!(error, GatewayError::ConnectionLost { .. }) {
                    if let Some(revoked) = self.revoked_after_loss(target).await {
                        return Err(revoked);
                    }
                }
                Err(error)
            }
        }
    }

    /// A wait that can outlast the relay token (a person answering a popup takes minutes) renews
    /// the token on the open connection first and then before each expiry; a short request does not.
    async fn request_keeping_token_fresh(
        &self,
        link: &Link,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
        target: &str,
        request_id: String,
        plaintext: &[u8],
        deadline: Duration,
    ) -> Result<Vec<u8>, LinkFailure> {
        let request = link.request(request_id, plaintext, deadline);
        let ends_at = std::time::Instant::now() + deadline;
        tokio::pin!(request);
        loop {
            let mut due = link.token_renewal_due(subscription::RENEW_EVERY);
            if ends_at <= due {
                // The token outlives this request: nothing to renew.
                return request.await;
            }
            tokio::select! {
                answered = &mut request => return answered,
                _ = tokio::time::sleep_until(due.into()) => {
                    if !self.renew_once(target, kernel, link).await {
                        tracing::warn!(target = %target, "could not renew the relay token of a long wait; it may be cut at expiry");
                        // Retried shortly, still inside the token's last minute and a half.
                        due = std::time::Instant::now() + subscription::RENEW_RETRY_AFTER;
                        tokio::select! {
                            answered = &mut request => return answered,
                            _ = tokio::time::sleep_until(due.into()) => {}
                        }
                    }
                }
            }
        }
    }

    /// Resolving the kernel, registering and opening the link wait on the network; access that
    /// ended meanwhile must not start a request on the target. Checked before and after.
    fn ensure_access_live(&self, origin: &Origin, target: &str) -> Result<(), GatewayError> {
        if self
            .runtime_state
            .access_grant_summary(&origin.grant_id)
            .is_none()
        {
            return Err(GatewayError::AccessEnded {
                name: target.into(),
            });
        }
        Ok(())
    }

    async fn link_for(
        &self,
        target: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
    ) -> Result<Arc<Link>, GatewayError> {
        let mut links = self.links.lock().await;
        if let Some(link) = links.get(&kernel.kernel_id).filter(|link| link.usable()) {
            return Ok(Arc::clone(link));
        }
        let link = self.open_link(target, kernel).await?;
        links.insert(kernel.kernel_id.clone(), Arc::clone(&link));
        Ok(link)
    }

    /// Asks `target` whether it still accepts this gateway; `Some` with the definitive error when
    /// the owner revoked it there. Any other failure says nothing about acceptance.
    async fn unaccepted(&self, target: &str, origin: &Origin) -> Option<GatewayError> {
        let probe =
            LocalDaemonRequest::HeartbeatAttachments(crate::local::HeartbeatAttachmentsRequest {
                attachments: Vec::new(),
            });
        let command_id = format!("gw-probe-{}", crate::session::unix_epoch_ms());
        match self
            .forward(target, origin, &command_id, &probe, REVOCATION_PROBE)
            .await
        {
            Err(error) if error.code() == "kernel_unpaired" => Some(error),
            _ => None,
        }
    }

    /// Resolves with the error as soon as `target` stops accepting this gateway. Used beside a
    /// request or a watch that can wait for minutes: the relay does not cut a self-hosted link
    /// when the owner revokes the gateway, so the origin asks every few seconds.
    pub(crate) async fn until_unaccepted(&self, target: &str, origin: &Origin) -> GatewayError {
        let mut tick = tokio::time::interval_at(
            tokio::time::Instant::now() + REVOCATION_PROBE,
            REVOCATION_PROBE,
        );
        loop {
            tick.tick().await;
            if let Some(error) = self.unaccepted(target, origin).await {
                return error;
            }
        }
    }

    /// A revoked gateway finds out because the relay cuts its link. A lost link is therefore
    /// answered by asking for a new one: when the owner revoked the gateway, that says so
    /// definitively instead of leaving a "retry" that can only fail. Otherwise the new link
    /// is ready for the retry.
    pub(crate) async fn revoked_after_loss(&self, target: &str) -> Option<GatewayError> {
        let kernel = self.resolve(target).await.ok()?;
        match self.link_for(target, &kernel).await {
            Err(revoked @ GatewayError::GatewayRevoked) => Some(revoked),
            _ => None,
        }
    }

    /// The hosted gateway is listed in the Cloud clients list for as long as an access
    /// grant the user approved uses it; each grant registers it, which also brings back
    /// a gateway the user revoked there while no grant was using it.
    async fn register_for(&self, grant_id: &str, target: &str) -> Result<(), GatewayError> {
        let snapshot = self.config.snapshot();
        let Some(profile) = snapshot.cloud_relay else {
            return Ok(());
        };
        let identity = self.identity()?;
        self.hosted
            .ensure_registered(
                &profile,
                GATEWAY_CLIENT_ID,
                &identity.thumbprint,
                grant_id,
                snapshot.daemon_alias.as_deref(),
                target,
            )
            .await
    }

    /// A fresh connection to `kernel`. A self-hosted kernel is held to the key
    /// it had when it was paired, whatever the relay announces now.
    async fn open_link(
        &self,
        target: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
    ) -> Result<Arc<Link>, GatewayError> {
        let identity = self.identity()?;
        let connection = self.connection(target, kernel, &identity).await?;
        let pinned = match self.config.snapshot().cloud_relay {
            None => self
                .paired
                .find(&kernel.kernel_id)
                .map(|paired| paired.presence.public_key),
            Some(_) => None,
        };
        let link = Link::open(LinkParams {
            relay_url: connection.relay_url,
            auth_token: connection.relay_token,
            target_kernel_id: kernel.kernel_id.clone(),
            target_public_key: Some(pinned.unwrap_or_else(|| kernel.public_key.clone())),
            client_private_key: identity.private_key,
        })
        .await
        .map_err(|failure| map_open_failure(target, failure))?;
        Ok(Arc::new(link))
    }

    async fn connection(
        &self,
        target: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
        identity: &GatewayIdentity,
    ) -> Result<crate::local::KernelClientConnection, GatewayError> {
        // A scoped self-hosted relay admits this kernel as a client only with the
        // CLIENT token the person gave when pairing.
        let config = self.config.snapshot();
        if let (None, Some(relay_url), Some(token)) = (
            &config.cloud_relay,
            &config.relay_url,
            self.paired
                .find(&kernel.kernel_id)
                .and_then(|target| target.relay_token),
        ) {
            return Ok(crate::local::KernelClientConnection {
                relay_url: relay_url.clone(),
                relay_token: token,
                target_daemon_id: Some(kernel.kernel_id.clone()),
                target_daemon_alias: kernel.kernel_alias.clone(),
                token_expires_at: None,
                machine_id: Some(kernel.machine_id.clone()),
                kernel_id: Some(kernel.kernel_id.clone()),
            });
        }
        match crate::runtime::cloud_relay_connection_executor::kernel_client_connection(
            &self.runtime_state,
            &self.config,
            kernel,
            Some(GATEWAY_CLIENT_ID.to_string()),
            None,
            Some(identity.thumbprint.clone()),
        )
        .await
        {
            Ok(connection) => Ok(connection),
            Err(error) if hosted::is_revoked(&error) => Err(self.gateway_revoked().await),
            Err(error) => Err(GatewayError::Unreachable {
                name: target.into(),
                detail: error::plain_cause("the relay did not accept the connection", &error),
            }),
        }
    }

    /// The owner revoked this kernel's gateway in the Cloud clients list. That ends
    /// every agent access grant; a new grant registers the gateway again.
    async fn gateway_revoked(&self) -> GatewayError {
        if self.hosted.revoked().await {
            let hosted = self.hosted.clone();
            let runtime_state = self.runtime_state.clone();
            tokio::spawn(async move {
                // Ending the grants closes the agent's connection, so the answer to the
                // request that found the revoke has to be delivered before they end.
                tokio::time::sleep(GRANTS_END_AFTER_REVOKE).await;
                if let Err(error) =
                    runtime_state.revoke_kernel_access(None, None, "gateway_revoked")
                {
                    tracing::warn!(%error, "could not end agent access grants after the gateway was revoked");
                }
                hosted.reopen().await;
            });
        }
        GatewayError::GatewayRevoked
    }

    fn identity(&self) -> Result<GatewayIdentity, GatewayError> {
        let mut identity = self.identity.lock().expect("identity poisoned");
        if let Some(identity) = identity.as_ref() {
            return Ok(identity.clone());
        }
        let created = GatewayIdentity::load_or_create(&self.config.snapshot())?;
        *identity = Some(created.clone());
        Ok(created)
    }
}

/// Opening the link sends nothing to the kernel, so a connect that does not
/// finish is "unreachable", never a request timeout that invites a retry.
fn map_open_failure(target: &str, failure: LinkFailure) -> GatewayError {
    match failure {
        LinkFailure::TimedOut => GatewayError::Unreachable {
            name: target.into(),
            detail: "the relay did not complete the connection in time".into(),
        },
        failure => map_failure(target, failure),
    }
}

fn map_failure(target: &str, failure: LinkFailure) -> GatewayError {
    let name = target.to_string();
    match failure {
        LinkFailure::Closed => GatewayError::ConnectionLost { name },
        LinkFailure::TimedOut => GatewayError::RequestTimeout { name },
        LinkFailure::IdentityMismatch => GatewayError::Unreachable {
            name,
            detail: "the kernel answered with a different key than the one it registered".into(),
        },
        LinkFailure::Relay(error) => match error.code.as_str() {
            "target_not_connected" => GatewayError::Offline { name },
            // The relay drops a request the target never answered when the target disconnects.
            "target_disconnected" => GatewayError::ConnectionLost { name },
            "authorization_denied" if error.message.contains("unpaired or revoked") => {
                GatewayError::Unpaired { name }
            }
            // The kernel could not decrypt the request: it restarted with another key since
            // this connection pinned the old one. A new connection checks the key again.
            "invalid_request" if error.message.starts_with("invalid relay request payload") => {
                GatewayError::Unreachable {
                    name,
                    detail: "the kernel could not read the request; it may have restarted with a new key. Retry once".into(),
                }
            }
            "invalid_request" => GatewayError::Incompatible {
                name,
                // An older target lists every variant it knows; the first part says what is wrong.
                detail: error.message.chars().take(240).collect(),
            },
            "connect_failed" | "connect_refused" | "empty_response" | "decrypt_failed"
            | "encrypt_failed" => GatewayError::Unreachable {
                name,
                detail: error.message,
            },
            _ => GatewayError::Remote {
                name,
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // EA2-11: the target's popup names the agent, not the file the origin happened to run
    // ("2.1.296" for Claude Code, "node" for Codex).
    #[test]
    fn a_grant_holder_is_named_by_its_harness_before_its_executable() {
        use crate::local::{KernelAccessGrant, KernelAccessProviderHarness, KernelAccessTerm};
        let grant = |executable: &str, harness| KernelAccessGrant {
            grant_id: "g".into(),
            owner_user_id: "local".into(),
            holder_pid: 1,
            holder_executable: executable.into(),
            holder_harness: harness,
            term: KernelAccessTerm::UntilHolderExits,
            expires_at_ms: None,
        };
        assert_eq!(
            harness_name(&grant(
                "/home/u/.local/share/claude/versions/2.1.296",
                Some(KernelAccessProviderHarness::Claude)
            )),
            "claude"
        );
        assert_eq!(
            harness_name(&grant(
                "/usr/bin/node",
                Some(KernelAccessProviderHarness::Codex)
            )),
            "codex"
        );
        assert_eq!(harness_name(&grant("/usr/bin/aider", None)), "aider");
    }

    // A request whose grant ended is not sent: it never gets as far as resolving the kernel.
    #[tokio::test]
    async fn a_request_under_an_ended_grant_is_not_forwarded() {
        let app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(app)),
            32,
        );
        let origin = Origin {
            grant_id: "ended-grant".into(),
            harness: "claude".into(),
        };
        let request = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
        let error = router
            .kernel_gateway()
            .forward("b3", &origin, "cmd-1", &request, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "kernel_access_ended");
        assert!(error.message().contains("nothing was sent"));
    }

    // Revoking a gateway ends its event streams too, not only its requests.
    #[tokio::test]
    async fn a_revoked_gateway_key_gets_no_event_stream() {
        let config = crate::config::DaemonConfig::for_tests();
        let app = crate::test_support::bootstrap_authenticated_app(config.clone()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(app)),
            32,
        );
        let key = "gateway-relay-public-key";
        let gateway = router.kernel_gateway();
        assert!(gateway
            .admit_origin(
                key,
                Some(origins::Named {
                    kernel: "kernel-a",
                    name: None
                })
            )
            .unwrap());
        router
            .admit_relay_stream(key)
            .expect("a live gateway streams");
        let client_id = format!(
            "{}{}",
            origins::CLIENT_PREFIX,
            crate::runtime::terminal_pairings::public_key_thumbprint(key)
        );
        let path = origins::path_in(&config.private_runtime_state_root());
        origins::revoke(&path, &client_id).unwrap().unwrap();
        router
            .admit_relay_stream(key)
            .expect_err("a revoked gateway gets no stream");
    }

    // EA2-09(c): with the relay stalled, a kernel without an open link is
    // unreachable within the connect bound; an open link that stalls times out.
    #[test]
    fn a_stalled_connect_is_unreachable_and_a_stalled_request_times_out() {
        assert_eq!(
            map_open_failure("b3", LinkFailure::TimedOut).code(),
            "kernel_unreachable"
        );
        assert_eq!(
            map_failure("b3", LinkFailure::TimedOut).code(),
            "kernel_request_timeout"
        );
        assert_eq!(
            map_open_failure("b3", LinkFailure::Closed).code(),
            "kernel_connection_lost"
        );
        // A target that restarted with a new key cannot read an old pinned request: retry, not "update".
        let unreadable = chariox_relay::protocol::RelayError {
            code: "invalid_request".into(),
            message: "invalid relay request payload: aead::Error".into(),
            retryable: false,
        };
        assert_eq!(
            map_failure("b3", LinkFailure::Relay(unreadable)).code(),
            "kernel_unreachable"
        );
        // EA2-09(a): the target killed with a request in flight is a lost connection, not a raw relay code.
        let disconnected = chariox_relay::protocol::RelayError {
            code: "target_disconnected".into(),
            message: "the target disconnected".into(),
            retryable: true,
        };
        assert_eq!(
            map_failure("b3", LinkFailure::Relay(disconnected)).code(),
            "kernel_connection_lost"
        );
    }
}
