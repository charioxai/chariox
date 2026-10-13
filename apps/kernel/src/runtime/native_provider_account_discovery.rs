//! MP-08 / MP-11: surfaces host-native provider logins in the account registry
//! without a "Use native account" step. A pass registers each installed
//! harness's native scope that holds a login, observes it with the official
//! CLI status command, and prefers it as the default when the current default
//! cannot start work. Passes run in the background, at most every 30 seconds,
//! and only for the provider-account authority owner.

use std::path::PathBuf;
use std::time::Duration;

use crate::account_profile::ProviderAccountProfileRegistry;
use crate::runtime::state::KernelRuntimeState;

const NATIVE_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) fn schedule_native_provider_account_discovery(runtime_state: &KernelRuntimeState) {
    // Unit-test kernels must not probe the developer's own native logins;
    // `discover_native_provider_accounts` is exercised directly instead.
    if cfg!(test) {
        return;
    }
    let registry = runtime_state.provider_account_profile_registry().clone();
    if !registry.begin_native_discovery_pass(NATIVE_DISCOVERY_INTERVAL) {
        return;
    }
    let owner = runtime_state
        .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
    let runtime_state = runtime_state.clone();
    tokio::spawn(async move {
        let pass_registry = registry.clone();
        let changed = tokio::task::spawn_blocking(move || {
            discover_native_provider_accounts(
                &pass_registry,
                &owner,
                &crate::provider::installed_provider_harnesses(),
            )
        })
        .await
        .unwrap_or(false);
        registry.finish_native_discovery_pass();
        if changed {
            runtime_state
                .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            runtime_state.record_waiting_room_change();
        }
    });
}

/// Returns whether any registration or observation changed.
pub(crate) fn discover_native_provider_accounts(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    installed_providers: &[String],
) -> bool {
    let Some(home) = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    else {
        return false;
    };
    let mut changed = false;
    for provider in installed_providers {
        let (profile, fingerprint) =
            match registry.native_scope_needing_observation(owner_user_id, provider, &home) {
                Ok(Some(candidate)) => candidate,
                Ok(None) => continue,
                Err(error) => {
                    log_discovery_error(provider, "register", &error);
                    continue;
                }
            };
        changed = true;
        match crate::local::provider_requests::observe_provider_auth_status(
            registry,
            owner_user_id,
            provider,
            &profile.profile_id,
        ) {
            Ok(_) => {
                if let Err(error) =
                    registry.prefer_authenticated_default(owner_user_id, provider, &profile.profile_id)
                {
                    log_discovery_error(provider, "default", &error);
                }
            }
            // A failed probe is retried when the native login changes or the
            // kernel restarts, not on every waiting-room refresh.
            Err(error) => log_discovery_error(provider, "observe", &error),
        }
        registry.record_native_scope_observed(owner_user_id, provider, fingerprint);
    }
    changed
}

fn log_discovery_error(provider: &str, stage: &str, error: &crate::error::DaemonError) {
    crate::logging::warn_with_fields(
        "daemon.provider_accounts",
        "native provider account discovery step failed",
        serde_json::json!({ "provider": provider, "stage": stage, "error": error.to_string() }),
    );
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn mp08_discovery_shows_a_native_claude_login_signed_in_and_default() {
        use std::os::unix::fs::PermissionsExt;
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-native-discovery-claude-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        // Synthetic login marker; the fake official CLI below reports status.
        std::fs::write(home.join(".claude").join(".credentials.json"), "{}").unwrap();
        let claude = root.join("claude");
        std::fs::write(
            &claude,
            "#!/bin/sh\nprintf '%s\\n' '{\"loggedIn\":true,\"email\":\"native@example.test\",\"subscriptionType\":\"max\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", &claude);
        std::env::set_var("HOME", &home);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let managed = registry.create_managed("local", "claude", "work").unwrap();

        assert!(super::discover_native_provider_accounts(&registry, "local", &["claude".to_string()]));

        let profiles = registry.list("local", Some("claude")).unwrap();
        let native = profiles
            .iter()
            .find(|profile| profile.profile_id != managed.profile_id)
            .expect("the native Claude login appears without an import step");
        assert_eq!(
            native.auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
        assert_eq!(native.identity_summary.as_deref(), Some("native@example.test"));
        assert!(native.is_default, "the signed-in native account replaces the unusable default");
        assert!(
            !super::discover_native_provider_accounts(&registry, "local", &["claude".to_string()]),
            "an unchanged native login is not probed again"
        );
        assert!(!super::discover_native_provider_accounts(&registry, "local", &[]));
    }
}
