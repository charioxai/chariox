//! MP-08 / MP-10 / MP-11: warm a new worker account through the official CLI.
//! OpenCode can expose its bundled snapshot before its background models.dev
//! refresh completes. Await its native refresh before acknowledging a replica.
use crate::account_profile::{ProviderAccountAuthState, ProviderAccountService};
use crate::error::DaemonError;
use std::collections::BTreeMap;
use std::time::Duration;

pub(crate) fn prepare_opencode_model_cache(
    environment: &BTreeMap<String, String>,
    services: &[ProviderAccountService],
) -> Result<(), DaemonError> {
    let executable = crate::provider::resolve_opencode_executable()?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["models".into(), "--refresh".into()],
        environment.clone(),
        None,
        "opencode:models-refresh",
    )?;
    super::remove_account_auth_environment(&mut command, "opencode");
    let failure = |message: String| DaemonError::LocalTransport {
        operation: "prepare_opencode_model_cache",
        message,
    };
    let mut output = super::probe_capture::capture(command, Duration::from_secs(60))
        .map_err(|_| failure("OpenCode model-cache refresh did not complete; retry account provisioning when its native catalog is reachable".into()))?;
    let result = if output.status.success() {
        validate_connected_models(&output.stdout, services).map_err(failure)
    } else {
        Err(failure(format!(
            "OpenCode model-cache refresh failed ({}); no worker catalog was admitted",
            output.status
        )))
    };
    // Provider stderr is never an operator diagnostic; native utilities may
    // include credential material even when this command normally lists models.
    zeroize::Zeroize::zeroize(&mut output.stdout);
    zeroize::Zeroize::zeroize(&mut output.stderr);
    result
}

fn validate_connected_models(
    output: &[u8],
    services: &[ProviderAccountService],
) -> Result<(), String> {
    let models = String::from_utf8_lossy(output);
    // Custom service definitions are not transferred with the native auth store.
    // Only the built-in Go/Zen catalog readiness gates worker admission here.
    for service in services.iter().filter(|s| {
        s.auth_state == ProviderAccountAuthState::Authenticated
            && matches!(s.service_id.as_str(), "opencode-go" | "opencode")
    }) {
        let prefix = format!("{}/", service.service_id);
        if !models.lines().any(|line| line.trim().starts_with(&prefix)) {
            return Err(format!("OpenCode native refresh still has no models for connected service `{}`; retry provisioning after its catalog becomes available", service.service_id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(id: &str, state: ProviderAccountAuthState) -> ProviderAccountService {
        ProviderAccountService {
            service_id: id.into(),
            label: id.into(),
            auth_state: state,
            credential_type: crate::account_profile::ProviderAccountServiceCredentialType::ApiKey,
            billing_kind: None,
        }
    }

    #[test]
    fn cached_openai_only_snapshot_cannot_admit_a_connected_go_account() {
        let services = [
            service("openai", ProviderAccountAuthState::Authenticated),
            service("opencode-go", ProviderAccountAuthState::Authenticated),
        ];
        assert!(validate_connected_models(b"openai/gpt-6.1-sol\n", &services).is_err());
        assert!(validate_connected_models(
            b"openai/gpt-6.1-sol\nopencode-go/claude-haiku-5-5\n",
            &services
        )
        .is_ok());
    }

    #[test]
    fn refresh_requires_exact_connected_service_without_requiring_logged_out_services() {
        let services = [
            service("opencode-go", ProviderAccountAuthState::Authenticated),
            service("opencode", ProviderAccountAuthState::NotConfigured),
        ];
        assert!(validate_connected_models(b"opencode-go-other/model\n", &services).is_err());
        assert!(validate_connected_models(b"opencode-go/gpt-6-luna\n", &services).is_ok());
    }
    #[test]
    fn r5_review_extra_custom_service_does_not_block_a_listed_go_account() {
        let services = [
            service("opencode-go", ProviderAccountAuthState::Authenticated),
            service("custom-home-only", ProviderAccountAuthState::Authenticated),
        ];
        assert!(validate_connected_models(b"opencode-go/gpt-6-luna\n", &services).is_ok());
        assert!(validate_connected_models(b"custom-home-only/model\n", &services).is_err());
    }
}
