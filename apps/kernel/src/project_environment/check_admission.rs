//! MP-08 / MP-10 / MP-11: named admission probes; Check does not repair or authorize.
use super::check::{outcome, EnvironmentCheckContext};
use super::*;
use EnvironmentObservationStatus::{Conflict, Missing, NeedsYourInput, Ready};
pub(super) fn check_admission(
    ctx: &EnvironmentCheckContext,
    r: &Requirement,
    deadline: std::time::Instant,
) -> (EnvironmentObservationStatus, String, String) {
    match &r.spec {
        RequirementSpec::Accounts {
            provider,
            linked_profile_ref,
            required_capabilities,
        } => {
            let family = match provider {
                EnvironmentProvider::Codex => "codex",
                EnvironmentProvider::Claude => "claude",
                EnvironmentProvider::OpenCode => "opencode",
            };
            let owner = crate::account_profile::provider_account_authority_owner_user_id(
                &ctx.config,
                &ctx.owner,
            );
            let Some(profile) = linked_profile_ref else {
                return outcome(
                    NeedsYourInput,
                    "provider_link_required",
                    "Link an official provider account through Provider Accounts",
                );
            };
            let Ok(mut p) = ctx.profiles.get(&owner, family, profile) else {
                return outcome(
                    NeedsYourInput,
                    "provider_link_required",
                    "Required provider account is unavailable",
                );
            };
            // Reuse the official read-only auth service, never login or a task.
            // A Vault enrollment has no native login to inspect; its last
            // verified result must remain explicit and time-bounded.
            let vault_enrolled = family == "claude"
                && crate::provider::provider_account_credential_registered(&owner, family, profile)
                    .unwrap_or(false);
            if !vault_enrolled {
                if crate::local::provider_requests::observe_provider_auth_status_until(
                    &ctx.profiles,
                    &owner,
                    family,
                    profile,
                    &ctx.config,
                    Some(deadline),
                )
                .is_err()
                {
                    return outcome(
                        NeedsYourInput,
                        "provider_status_unavailable",
                        "Official provider account status is unavailable; no login or turn started",
                    );
                }
                let Ok(updated) = ctx.profiles.get(&owner, family, profile) else {
                    return outcome(
                        NeedsYourInput,
                        "provider_status_unavailable",
                        "Provider account observation unavailable",
                    );
                };
                p = updated;
            }
            if p.auth_state != crate::account_profile::ProviderAccountAuthState::Authenticated
                || p.last_validated_at_ms.is_none_or(|t| {
                    t > crate::session::unix_epoch_ms()
                        || crate::session::unix_epoch_ms().saturating_sub(t) > 60_000
                })
            {
                return outcome(
                    NeedsYourInput,
                    "provider_authorization_stale",
                    "Provider authorization needs a fresh native account status check",
                );
            }
            if required_capabilities.iter().any(|id| {
                !p.services.iter().any(|s| {
                    &s.service_id == id
                        && s.auth_state
                            == crate::account_profile::ProviderAccountAuthState::Authenticated
                })
            }) {
                return outcome(NeedsYourInput,"provider_capability_unproven","Required provider capability is absent or unauthorized in the official account service");
            }
            outcome(
                Ready,
                "provider_authorized",
                "Current official provider authorization is available",
            )
        }
        RequirementSpec::CharioxApps {
            app_id,
            version,
            content_digest,
            required_grants,
        } => {
            let Ok(installations) = super::environment_app_installations(&ctx.apps, &ctx.owner)
            else {
                return outcome(
                    NeedsYourInput,
                    "app_registry_unavailable",
                    "App admission registry unavailable",
                );
            };
            let Some(a) = installations.iter().find(|a| a.app_id == *app_id) else {
                return outcome(
                    Missing,
                    "app_absent",
                    "Required Chariox App is not installed",
                );
            };
            let Some(active) = &a.active else {
                return outcome(
                    Missing,
                    "app_inactive",
                    "App has no active admitted generation",
                );
            };
            if content_digest
                .as_ref()
                .is_some_and(|d| d != &active.release.package_digest)
                || version
                    .as_ref()
                    .is_some_and(|v| v != &active.release.version)
            {
                return outcome(
                    Conflict,
                    "app_identity_conflict",
                    "Active App release differs from the required reviewed identity",
                );
            }
            if ctx
                .apps
                .active_app_capabilities(&ctx.owner, &a.installation_id)
                .is_err()
            {
                return outcome(
                    Conflict,
                    "app_signature_invalid",
                    "Active App package admission/signature is no longer verified",
                );
            }
            let grants = ctx
                .apps
                .app_connection_grants(&ctx.owner, &a.installation_id);
            let grants_verified = required_grants.iter().all(|name| {
                grants.as_ref().is_ok_and(|grants| {
                    grants.iter().any(|g| {
                        name == &format!("connection:{}:{}", g.generator_id, g.connection_id)
                    })
                })
            });
            if a.admission_paused || !grants_verified {
                return outcome(
                    NeedsYourInput,
                    "app_grants_required",
                    "Required App admission/grants need current capability-service verification",
                );
            }
            outcome(
                Ready,
                "app_admitted",
                "Current App generation is installed and admitted",
            )
        }
        _ => unreachable!("named Check dispatch"),
    }
}
