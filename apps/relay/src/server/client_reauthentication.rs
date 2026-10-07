use crate::auth::VerifiedRelayIdentity;

pub(super) fn same_client(previous: &VerifiedRelayIdentity, fresh: &VerifiedRelayIdentity) -> bool {
    previous.realm_id == fresh.realm_id
        && previous.subject == fresh.subject
        && previous.subject_kind == fresh.subject_kind
        && previous.account_id == fresh.account_id
        && previous.user_id == fresh.user_id
        && previous.client_id == fresh.client_id
        && previous.public_key_thumbprint == fresh.public_key_thumbprint
}

// Retained request/subscription routes were admitted using the old actions.
// Close on reduction rather than let those routes outlive their authorization.
pub(super) fn retains_permissions(
    previous: &VerifiedRelayIdentity,
    fresh: &VerifiedRelayIdentity,
) -> bool {
    previous
        .allowed_actions
        .iter()
        .all(|action| fresh.allowed_actions.contains(action))
}
