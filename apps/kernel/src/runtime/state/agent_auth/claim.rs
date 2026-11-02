//! One kernel-owned owner card; service completion, not a button, proves ownership.
//! Claim tokens remain in this bounded future and are discarded on cancel/restart.
use super::registration::{token_credential, Secret, TokenReply};
use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};

#[derive(serde::Deserialize)]
pub(super) struct Ceremony {
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

struct ClaimCard {
    state: KernelRuntimeState,
    session_id: String,
    id: String,
}
impl Drop for ClaimCard {
    fn drop(&mut self) {
        let _ = self
            .state
            .owned
            .timeout_runtime_interaction(&self.session_id, &self.id);
    }
}

pub(super) async fn complete(
    http: &AuthHttp,
    discovery: &Discovery,
    args: &RegisterArgs,
    registration_id: String,
    claim_token: Secret,
    ceremony: Ceremony,
) -> Result<IssuedCredential, DaemonError> {
    let verification_uri = http::endpoint(&ceremony.verification_uri)?;
    if ceremony.user_code.len() != 6
        || !ceremony.user_code.bytes().all(|byte| byte.is_ascii_digit())
        || ceremony.expires_in == 0
    {
        return Err(error(
            "auth.md service returned invalid claim ceremony materials",
        ));
    }
    if http.authority().resource.scheme() == "https" && verification_uri.scheme() != "https" {
        return Err(error("auth.md claim cannot downgrade HTTPS to HTTP"));
    }
    let endpoint = discovery
        .token_endpoint
        .as_deref()
        .ok_or_else(|| error("auth.md claim requires token_endpoint metadata"))?;
    let authority = http.authority();
    authority.check()?;
    let id = format!("agent-auth-claim-{:032x}", rand::random::<u128>());
    let expires_in = ceremony.expires_in.min(3600);
    let interaction = RuntimeInteraction::for_kernel_operation(
        &id,
        &id,
        "Claim service account",
        format!(
            "Open this link, sign in and enter code {}:\n{}",
            ceremony.user_code, verification_uri
        ),
        vec![RuntimeInteractionChoice::new(
            "cancel", "Cancel", "cancel", None,
        )],
    )
    .with_timeout_sec(expires_in);
    let mut resolution = authority
        .state
        .create_kernel_operation_interaction(
            &authority.actor.session_id,
            &authority.actor.owner_user_id,
            interaction,
        )
        .await?;
    let _card = ClaimCard {
        state: authority.state.clone(),
        session_id: authority.actor.session_id.clone(),
        id,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    let mut interval = ceremony.interval.unwrap_or(5).clamp(1, 3600);
    let mut wait = interval;
    loop {
        tokio::select! {
            _ = &mut resolution => return Err(error("auth.md claim was cancelled or expired; restart registration when ready")),
            _ = tokio::time::sleep_until(deadline) => return Err(error("auth.md claim expired; restart registration for a new code")),
            _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(error(
                "auth.md claim expired; restart registration for a new code",
            ));
        }
        authority.check()?;
        let pairs = [
            ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
            ("claim_token", claim_token.as_str()),
        ];
        let reply = tokio::select! {
            _ = &mut resolution => return Err(error("auth.md claim was cancelled; restart registration when ready")),
            _ = tokio::time::sleep_until(deadline) => return Err(error("auth.md claim expired; restart registration for a new code")),
            result = http.post_form(endpoint, &pairs) => result?,
        };
        if reply.status == 429 {
            wait = reply
                .retry_after
                .unwrap_or(interval)
                .max(interval)
                .min(3600);
            continue;
        }
        if reply.status >= 500 {
            wait = wait.saturating_mul(2).max(interval).min(3600);
            continue;
        }
        let token: TokenReply = reply.json()?;
        match token.error.as_deref() {
            Some("authorization_pending") => wait = interval,
            Some("slow_down") => { interval = interval.saturating_add(5); wait = interval; }
            Some("expired_token" | "claim_expired" | "invalid_claim_token") => return Err(error("auth.md claim expired; restart registration for a new code")),
            Some(_) => return Err(error("auth.md claim was rejected by the service; consult its documentation before restarting")),
            None => {
                reply.success()?;
                authority.check()?;
                return token_credential(token, registration_id, &discovery.resource, &args.scopes);
            }
        }
    }
}
