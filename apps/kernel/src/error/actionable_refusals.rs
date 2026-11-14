//! MP-08 / MP-10 / MP-11: constant remediations safe at every external boundary.
pub(crate) fn message(code: &str) -> Option<&'static str> {
    if crate::provider_account_copy::is_public_code(code) {
        return Some(crate::provider_account_copy::failure_message(code));
    }
    Some(match code {
        "control_plane_grant_required" => "Use the owner's Chariox access grant for machine requests",
        "control_plane_unavailable" => "This kernel has no connected machine service",
        "control_plane_authorization_denied" => "Reconnect the owner's signed-in client and refresh machine capabilities",
        "control_plane_binding_invalid" => "The connected service returned an invalid machine binding",

        "provider_profile_unverified" => "provider profile has not been verified; run RefreshProviderAccountProfile {provider, account_profile} on the executing kernel, then retry the prompt",
        "provider_profile_unauthenticated" => "provider profile is not authenticated; run RefreshProviderAccountProfile {provider, account_profile} on the executing kernel. If it remains unauthenticated, reconnect it in Provider Accounts before retrying",
        "provider_profile_usage_exhausted" => "provider profile usage is exhausted; run RefreshProviderAccountProfile {provider, account_profile} to refresh usage, or select another account before starting work",
        "provider_profile_unavailable" => "provider profile is not registered for this owner on this kernel; run ListProviderAccountProfiles {provider}, then link or select a profile on the executing kernel",
        "provider_account_name_taken" => "an account with this name already exists on this kernel; choose another name or use the existing account",
        "provider_login_code_incomplete" => "the sign-in code is incomplete; paste the whole code the provider showed and submit it again",
        "provider_login_code_invalid" => "the provider rejected the sign-in code; start the sign-in again and paste the new code",
        "agent_not_in_session" => "agent does not belong to this Room; run ListAgents {session_id} and use the Room that owns the agent",
        "agent_not_found" => "agent not found; run ListAgents {session_id} in the intended Room and use its returned agent id",
        "attachment_not_found" => "attachment not found; run AttachToSession {session_id, client_id, capability_level} with --command-id ID and use SessionAttached.attachment.id",
        "attachment_not_in_session" => "attachment belongs to another Room; run AttachToSession {session_id, client_id, capability_level} with --command-id ID in the intended Room and use its returned attachment id",
        "no_active_prompt" => "agent has no matching active prompt; run GetSessionState {session_id} and use its current prompt id. Do not turn a rejected steer into a new prompt",
        "prompt_steer_rejected" => "active prompt changed or did not accept steering; run GetSessionState {session_id} and steer only with the expected current prompt id. Do not submit a replacement prompt automatically",
        "prompt_steer_event_unrecorded" => "the steer was delivered but its event was not recorded; do not send it again",
        "heartbeat_interval_not_routable" => "a subscription routed to another kernel uses that kernel's heartbeat interval; leave heartbeat_interval_ms out of the subscribe frame",
        "secret_link_unavailable" => "this kernel has no page for secret links; ask the user to answer in a connected Chariox client, or to set secret_link.page_url",
        "vault_locked" => "the user's Vault is locked; ask them to unlock it in a Chariox client, then ask again",
        "target_vault_locked" => "this kernel's Vault is locked; ask the user to unlock it in a Chariox client, then repeat the same request. Nothing was imported",
        "secret_request_exists" => "this id already has a credential or a stored value; ask again with overwrite true (--overwrite) to replace the value, or choose another id",
        "secret_request_invalid" => "the credential id is letters, digits, - and _ separated by dots, the label is one line of 1 to 80 characters, and the key needs the API host(s) that may receive it, for example `--host api.example.com` (an existing handle keeps the hosts it has)",
        "secret_request_denied" => "the user's settings do not let agents create Vault credentials on this kernel",
        "secret_request_unknown" => "this agent has no such secret request; use the interaction_id that chariox secret request printed",
        "too_many_open_files" => "too many open files in this kernel; close a terminal and try again",
        "agent_deadline_reached" => "agent deadline reached; this agent cannot start another turn. Start a new agent for new work",
        "agent_deadline_requires_executor" => "arm an agent deadline on its executing kernel; an agent with a deadline cannot move to a leased worker without worker enforcement",
        "agent_deadline_invalid" => "deadline must be a positive bounded epoch millisecond, or null to clear; supply deadline_ms to SetAgentDeadline",
        "agent_deadline_cleanup_pending" => "deadline reached; physical cleanup is pending and will retry. Read GetAgentDeadline for the stopping receipt",
        _ => return None,
    })
}
