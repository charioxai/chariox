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
        let authority = self.refresh_authority();
        let now = crate::session::unix_epoch_ms();
        let mut leases = self.leases.lock().expect("local browser leases poisoned");
        let lease = leases.get_mut(id).ok_or_else(denied)?;
        let original = &lease.grant.identity;
        // Cloud bootstrap can issue a new scoped subject. The same browser key,
        // user and realm must still prove admission over the relay. The original
        // latest short identity bounds this connection, never the browser timer.
        if identity.subject_kind != RelaySubjectKind::Client
            || identity.user_id.is_none()
            || identity.user_id != original.user_id
            || identity.realm_id != original.realm_id
            || identity.public_key_thumbprint != original.public_key_thumbprint
            || identity.expires_at_ms <= now
            || identity.expires_at_ms > now.saturating_add(GRANT_TTL_MS)
            || authority.as_ref() != Some(&lease.grant.authority)
            || lease.expiry.borrow().expires_at_ms <= now
            || sequence != lease.sequence
        {
            return Err(denied());
        }
        let expires_at_ms = (now + GRANT_TTL_MS).min(identity.expires_at_ms);
        lease.sequence = sequence.checked_add(1).ok_or_else(denied)?;
        lease.expiry.send_replace(LocalBrowserLeaseState {
            expires_at_ms,
            identity: identity.clone(),
        });
        Ok(
            serde_json::json!({"LocalBrowserLeaseRenewed": {"expires_at_ms": expires_at_ms, "next_sequence": lease.sequence}}),
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
