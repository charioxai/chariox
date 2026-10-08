use crate::auth::VerifiedRelayIdentity;

// Reuse client_connect for in-place renewal. Existing routes may remain only
// for the same identity/key with all of their packet-routing permission.
pub(super) fn retains_client_authority(
    previous: &VerifiedRelayIdentity,
    next: &VerifiedRelayIdentity,
) -> bool {
    previous.realm_id == next.realm_id
        && previous.subject == next.subject
        && previous.subject_kind == next.subject_kind
        && previous.account_id == next.account_id
        && previous.user_id == next.user_id
        && previous.machine_id == next.machine_id
        && previous.client_id == next.client_id
        && previous.public_key_thumbprint == next.public_key_thumbprint
        && previous
            .allowed_actions
            .iter()
            .all(|action| next.allowed_actions.contains(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{RelayAction, RelaySubjectKind};
    #[test]
    fn renewal_preserves_identity_key_and_route_permissions() {
        let mut old = VerifiedRelayIdentity::bootstrap(RelayAction::ClientConnect);
        old.subject_kind = RelaySubjectKind::Client;
        old.public_key_thumbprint = Some("terminal-key".into());
        old.allowed_actions.push(RelayAction::PacketRoute);
        let mut fresh = old.clone(); // Expiry/token ID are replaced, not identity.
        fresh.token_id = Some("fresh-token".into());
        assert!(retains_client_authority(&old, &fresh));
        fresh.allowed_actions = vec![RelayAction::ClientConnect];
        assert!(!retains_client_authority(&old, &fresh));
        fresh = old.clone();
        fresh.public_key_thumbprint = Some("foreign-key".into());
        assert!(!retains_client_authority(&old, &fresh));
        fresh = old.clone();
        fresh.subject = "another-terminal".into();
        assert!(!retains_client_authority(&old, &fresh));
    }
}
