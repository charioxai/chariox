//! MP-08/MP-11: only relay-verified, key-bound callers can renew an active lease.
use super::*;

pub(super) struct LocalBrowserLease {
    grant: LocalBrowserGrant,
    sequence: u64,
    expiry: watch::Sender<LocalBrowserLeaseState>,
}

#[derive(Clone)]
pub(super) struct LocalBrowserLeaseState {
    pub(super) expires_at_ms: u64,
    pub(super) identity: RelayCallerIdentity,
}

impl LocalBrowserDirect {
    pub(super) fn activate_lease(
        &self,
        id: &str,
        grant: &LocalBrowserGrant,
    ) -> watch::Receiver<LocalBrowserLeaseState> {
        let (expiry, receiver) = watch::channel(LocalBrowserLeaseState {
            expires_at_ms: grant.expires_at_ms,
            identity: grant.identity.clone(),
        });
        self.leases
            .lock()
            .expect("local browser leases poisoned")
            .insert(
                id.to_owned(),
                LocalBrowserLease {
                    grant: grant.clone(),
                    sequence: 1,
                    expiry,
                },
            );
        receiver
    }

    pub(super) fn retire_lease(&self, id: &str) {
        self.leases
            .lock()
            .expect("local browser leases poisoned")
            .remove(id);
    }

    pub(in crate::transport::relay_client) fn renew_lease(
        &self,
        identity: &RelayCallerIdentity,
        id: &str,
        sequence: u64,
    ) -> Result<serde_json::Value, RelayError> {
        self.renew_client_lease(identity, id, sequence, LocalDirectClient::Browser)
    }

    pub(in crate::transport::relay_client) fn renew_terminal_lease(
        &self,
        identity: &RelayCallerIdentity,
        id: &str,
        sequence: u64,
    ) -> Result<serde_json::Value, RelayError> {
        self.renew_client_lease(identity, id, sequence, LocalDirectClient::Terminal)
    }

    fn renew_client_lease(
        &self,
        identity: &RelayCallerIdentity,
        id: &str,
        sequence: u64,
        client: LocalDirectClient,
    ) -> Result<serde_json::Value, RelayError> {
        let authority = self.refresh_authority();
        let now = crate::session::unix_epoch_ms();
        let mut leases = self.leases.lock().expect("local browser leases poisoned");
        let lease = leases.get_mut(id).ok_or_else(denied)?;
        let original = &lease.grant.identity;
        // Cloud bootstrap can issue a new scoped subject. The same browser key,
        // user and realm must still prove admission over the relay. The original
        // latest short identity bounds this connection, never the browser timer.
        if lease.grant.client != client
            || identity.subject_kind != RelaySubjectKind::Client
            || identity.user_id.is_none()
            || identity.user_id != original.user_id
            || identity.realm_id != original.realm_id
            || identity.public_key_thumbprint != original.public_key_thumbprint
            || authority.as_ref() != Some(&lease.grant.authority)
            || lease.expiry.borrow().expires_at_ms <= now
            || identity.expires_at_ms <= now
            // A higher sequence recovers a renewal whose response was lost;
            // every used sequence stays refused.
            || sequence < lease.sequence
        {
            return Err(denied());
        }
        let expires_at_ms = client_identity_deadline(identity, now, client).ok_or_else(|| {
            relay_error(
                "local_browser_lease_clock_skew",
                "local browser relay identity expires more than 40 seconds ahead of the kernel clock; check that the system clock is synchronized",
                false,
            )
        })?;
        lease.sequence = sequence.checked_add(1).ok_or_else(denied)?;
        lease.expiry.send_replace(LocalBrowserLeaseState {
            expires_at_ms,
            identity: identity.clone(),
        });
        let name = match client {
            LocalDirectClient::Browser => "LocalBrowserLeaseRenewed",
            LocalDirectClient::Terminal => "LocalTerminalLeaseRenewed",
        };
        Ok(
            serde_json::json!({name: {"expires_at_ms": expires_at_ms, "next_sequence": lease.sequence}}),
        )
    }
}

fn denied() -> RelayError {
    relay_error(
        "local_browser_lease_denied",
        "local browser lease is expired, retired, replayed or belongs to another caller",
        false,
    )
}
