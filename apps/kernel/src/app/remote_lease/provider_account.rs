use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountMaterialization, ProviderAccountProfile,
};
use crate::error::DaemonError;
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

use super::RemoteLeaseRuntime;

impl RemoteLeaseRuntime<'_> {
    pub(super) fn resolve_leased_profile_account(
        &self,
        leased_agent: &crate::execution_lease::LeasedAgent,
        provider: &str,
        account_profile: &str,
    ) -> Result<String, DaemonError> {
        if crate::provider::canonical_provider_family(provider).is_none() {
            return Ok(account_profile.to_string());
        }
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        // Replicas are registered under the lease owner, not the worker's local
        // user or another lease's owner. Resolve aliases before changing a run.
        self.app.provider_account_profile_registry()
            .get(&lease.owner_user_id, provider, account_profile)
            .map(|profile| profile.profile_id)
            .map_err(|_| DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: format!("the selected {provider} account is unavailable on the worker; connect or select an available account before changing the profile"),
            })
    }

    pub(crate) fn ensure_remote_provider_account(
        &mut self,
        context: RemoteProviderAccountSyncContext,
        mut materialization: ProviderAccountMaterialization,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&context.execution_lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: context.execution_lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message:
                    "provider-account materialization context does not match the execution lease"
                        .to_string(),
            });
        }
        if lease.owner_user_id != materialization.profile.owner_user_id {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message:
                    "provider-account materialization owner does not match the execution lease"
                        .to_string(),
            });
        }
        if crate::provider::canonical_provider_family(&materialization.profile.provider)
            == Some("claude")
            && materialization
                .files
                .iter()
                .any(|file| file.relative_path == ".credentials.json")
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message: "Claude provider credentials cannot be materialized on a remote worker; use the kernel-managed Chariox-vault setup-token launch path".to_string(),
            });
        }

        if let Some(profile) = self
            .app
            .provider_account_profile_registry()
            .list(
                &lease.owner_user_id,
                Some(&materialization.profile.provider),
            )?
            .into_iter()
            .find(|profile| profile.profile_id == materialization.profile.profile_id)
        {
            return self.validate_remote_provider_account(profile);
        }

        materialization.profile.origin =
            crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated;
        let profile = self
            .app
            .provider_account_profile_registry()
            .materialize_replica(&lease.owner_user_id, &materialization)?;
        self.app.durable_state_store().append_event(
            "provider_account.materialized",
            Some(lease.id),
            serde_json::json!({
                "owner_user_id": profile.owner_user_id,
                "provider": profile.provider,
                "profile_id": profile.profile_id,
                "source_home_kernel_id": context.home_kernel_id,
            }),
        )?;
        self.validate_remote_provider_account(profile)
    }

    fn validate_remote_provider_account(
        &self,
        profile: ProviderAccountProfile,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        if !remote_provider_account_requires_auth_validation(&profile.provider, profile.auth_state)
        {
            return Ok(profile);
        }
        let registry = self.app.provider_account_profile_registry();
        crate::local::provider_requests::observe_provider_auth_status(
            &registry,
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
        registry.require_authenticated(
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
            None,
            "ensure remote provider account",
        )
    }
}

fn remote_provider_account_requires_auth_validation(
    provider: &str,
    auth_state: ProviderAccountAuthState,
) -> bool {
    // Claude refresh credentials never cross this boundary. Its official CLI
    // is authenticated at launch through the existing vaulted setup-token
    // path, so preserve that separate handoff contract.
    crate::provider::canonical_provider_family(provider) != Some("claude")
        && auth_state != ProviderAccountAuthState::Authenticated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn install_opencode_auth_fixture(root: &std::path::Path) -> impl Drop {
        use std::os::unix::fs::PermissionsExt;

        struct Cleanup {
            root: std::path::PathBuf,
            previous_bin: Option<std::ffi::OsString>,
            previous_isolation: Option<std::ffi::OsString>,
        }

        impl Drop for Cleanup {
            fn drop(&mut self) {
                match self.previous_bin.take() {
                    Some(value) => std::env::set_var("CHARIOX_OPENCODE_BIN", value),
                    None => std::env::remove_var("CHARIOX_OPENCODE_BIN"),
                }
                match self.previous_isolation.take() {
                    Some(value) => {
                        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
                    }
                    None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        let executable = root.join("opencode");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 2 ] && [ "$1" = "auth" ] && [ "$2" = "list" ]; then
  test -f "$XDG_DATA_HOME/opencode/auth.json"
  printf '%s\n' '1 credential'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'opencode 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("OpenCode fixture should write");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("OpenCode fixture should be executable");
        let previous_bin = std::env::var_os("CHARIOX_OPENCODE_BIN");
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var("CHARIOX_OPENCODE_BIN", executable);
        std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        Cleanup {
            root: root.to_path_buf(),
            previous_bin,
            previous_isolation,
        }
    }

    #[cfg(unix)]
    fn install_claude_auth_fixture(root: &std::path::Path) -> impl Drop {
        use std::os::unix::fs::PermissionsExt;

        struct Cleanup {
            root: std::path::PathBuf,
            previous_bin: Option<std::ffi::OsString>,
            previous_isolation: Option<std::ffi::OsString>,
        }

        impl Drop for Cleanup {
            fn drop(&mut self) {
                match self.previous_bin.take() {
                    Some(value) => std::env::set_var("CHARIOX_CLAUDE_BIN", value),
                    None => std::env::remove_var("CHARIOX_CLAUDE_BIN"),
                }
                match self.previous_isolation.take() {
                    Some(value) => {
                        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
                    }
                    None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        let executable = root.join("claude");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' '{"loggedIn":true,"email":"worker@example.com","subscriptionType":"Pro"}'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude fixture 1.0.0'
  exit 0
fi
exit 2
"#,
        )
        .expect("Claude fixture should write");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("Claude fixture should be executable");
        let previous_bin = std::env::var_os("CHARIOX_CLAUDE_BIN");
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
        std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        Cleanup {
            root: root.to_path_buf(),
            previous_bin,
            previous_isolation,
        }
    }

    fn remote_account_fixture(
        root: &std::path::Path,
    ) -> (crate::DaemonApp, RemoteProviderAccountSyncContext) {
        std::fs::create_dir_all(root).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "owner-a",
            )
            .unwrap();
        let context = RemoteProviderAccountSyncContext {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            execution_lease_id: lease.id,
        };
        (app, context)
    }

    #[test]
    fn materialization_requires_matching_lease_owner_and_creates_profile_root() {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-account-materialization-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "owner-a",
            )
            .unwrap();
        let context = RemoteProviderAccountSyncContext {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            execution_lease_id: lease.id.clone(),
        };
        let materialization = ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner-a".to_string(),
                provider: "claude".to_string(),
                profile_id: "work".to_string(),
                label: "Work".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: Vec::new(),
            generated_at_ms: 1,
        };

        let profile = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context.clone(), materialization.clone())
            .unwrap();
        assert_eq!(profile.profile_id, "work");
        let environment = app
            .provider_account_profile_registry()
            .resolve_environment("owner-a", "claude", "work")
            .unwrap();
        assert!(std::path::Path::new(&environment["CLAUDE_CONFIG_DIR"]).is_dir());

        let claude_with_refresh_credential = ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner-a".to_string(),
                provider: "claude".to_string(),
                profile_id: "work".to_string(),
                label: "Work".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                relative_path: ".credentials.json".to_string(),
                contents_base64: "bmV2ZXItbG9nLXRoaXM=".to_string(),
            }],
            generated_at_ms: 1,
        };
        let error = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context.clone(), claude_with_refresh_credential)
            .expect_err("remote Claude refresh credentials must be rejected");
        assert!(error.to_string().contains("setup-token launch path"));
        assert!(!error.to_string().contains("bmV2ZXItbG9nLXRoaXM"));

        let mut wrong_owner = materialization;
        wrong_owner.profile.owner_user_id = "owner-b".to_string();
        assert!(RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context, wrong_owner)
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repeated_ensure_preserves_the_worker_owned_provider_profile() {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-account-handoff-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "owner-a",
            )
            .unwrap();
        let context = RemoteProviderAccountSyncContext {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            execution_lease_id: lease.id,
        };
        let materialization = |contents_base64: &str| ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner-a".to_string(),
                provider: "claude".to_string(),
                profile_id: "work".to_string(),
                label: "Work".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                relative_path: "settings.json".to_string(),
                contents_base64: contents_base64.to_string(),
            }],
            generated_at_ms: 1,
        };

        RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(
                context.clone(),
                materialization("eyJzb3VyY2UiOiJpbml0aWFsIn0="),
            )
            .unwrap();
        let environment = app
            .provider_account_profile_registry()
            .resolve_environment("owner-a", "claude", "work")
            .unwrap();
        let claude_config_dir = std::path::Path::new(&environment["CLAUDE_CONFIG_DIR"]);
        let settings_path = claude_config_dir.join("settings.json");
        let provider_state_path = claude_config_dir.join("provider-owned-state.json");
        std::fs::write(&settings_path, br#"{"source":"worker"}"#).unwrap();
        std::fs::write(&provider_state_path, b"worker-state").unwrap();

        RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(
                context,
                materialization("eyJzb3VyY2UiOiJyZWZyZXNoIn0="),
            )
            .unwrap();

        assert_eq!(
            std::fs::read(&settings_path).unwrap(),
            br#"{"source":"worker"}"#
        );
        assert_eq!(
            std::fs::read(&provider_state_path).unwrap(),
            b"worker-state"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_auth_validation_policy_preserves_codex_and_claude_handoffs() {
        assert!(remote_provider_account_requires_auth_validation(
            "codex",
            ProviderAccountAuthState::Unknown,
        ));
        assert!(!remote_provider_account_requires_auth_validation(
            "codex",
            ProviderAccountAuthState::Authenticated,
        ));
        assert!(!remote_provider_account_requires_auth_validation(
            "claude",
            ProviderAccountAuthState::Unknown,
        ));
    }

    #[cfg(unix)]
    #[test]
    fn fresh_claude_portable_materialization_is_accepted_private_and_native_validated() {
        use base64::Engine as _;
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-claude-auth-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let _cleanup = install_claude_auth_fixture(&root);
        let (mut app, context) = remote_account_fixture(&root);
        let materialization =
            |profile_id: &str, contents_base64: &str| ProviderAccountMaterialization {
                profile: crate::account_profile::ProviderAccountReplicaMetadata {
                    owner_user_id: "owner-a".to_string(),
                    provider: "claude".to_string(),
                    profile_id: profile_id.to_string(),
                    label: "Claude Work".to_string(),
                    origin: crate::account_profile::ProviderAccountProfileOrigin::Linked,
                    is_default: false,
                },
                files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                    relative_path: ".credentials.json".to_string(),
                    contents_base64: contents_base64.to_string(),
                }],
                generated_at_ms: 1,
            };
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);

        let portable = materialization(
            "claude-portable",
            &encode(br#"{"claudeAiOauth":{"refreshToken":"portable-refresh"}}"#),
        );
        assert!(!format!("{portable:?}").contains("portable-refresh"));

        let profile = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context.clone(), portable)
            .expect("portable Claude credentials must materialize on the worker");
        assert_eq!(
            profile.auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
        assert!(profile.last_validated_at_ms.is_some());

        let environment = app
            .provider_account_profile_registry()
            .resolve_environment("owner-a", "claude", "claude-portable")
            .unwrap();
        let claude_config_dir = std::path::Path::new(&environment["CLAUDE_CONFIG_DIR"]);
        let credentials_path = claude_config_dir.join(".credentials.json");
        assert_eq!(
            std::fs::read(&credentials_path).unwrap(),
            br#"{"claudeAiOauth":{"refreshToken":"portable-refresh"}}"#
        );
        assert_eq!(
            std::fs::metadata(&credentials_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let settings_path = claude_config_dir.join("settings.json");
        let provider_state_path = claude_config_dir.join("provider-owned-state.json");
        std::fs::write(&settings_path, br#"{"source":"worker"}"#).unwrap();
        std::fs::write(&provider_state_path, b"worker-state").unwrap();
        RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(
                context.clone(),
                materialization(
                    "claude-portable",
                    &encode(br#"{"claudeAiOauth":{"refreshToken":"replacement-refresh"}}"#),
                ),
            )
            .expect("repeated ensure keeps the worker-owned account");
        assert_eq!(
            std::fs::read(&settings_path).unwrap(),
            br#"{"source":"worker"}"#
        );
        assert_eq!(
            std::fs::read(&provider_state_path).unwrap(),
            b"worker-state"
        );

        for (profile_id, decoded, decoded_marker) in [
            ("claude-empty", b"".as_slice(), "claudeAiOauth"),
            (
                "claude-whitespace-refresh",
                br#"{"claudeAiOauth":{"refreshToken":" \t"}}"#.as_slice(),
                "refreshToken",
            ),
            (
                "claude-nonrefreshable",
                br#"{"claudeAiOauth":{"accessToken":"expired"}}"#.as_slice(),
                "accessToken",
            ),
        ] {
            let contents_base64 = encode(decoded);
            let error = RemoteLeaseRuntime::new(&mut app)
                .ensure_remote_provider_account(
                    context.clone(),
                    materialization(profile_id, &contents_base64),
                )
                .expect_err("non-portable Claude credentials must fail before provisioning");
            let message = error.to_string();
            assert!(!message.contains(decoded_marker), "{message}");
            if !contents_base64.is_empty() {
                assert!(!message.contains(&contents_base64), "{message}");
            }
            assert!(
                app.provider_account_profile_registry()
                    .get("owner-a", "claude", profile_id)
                    .is_err(),
                "{message}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn opencode_materialization_requires_native_authentication_before_acknowledgement() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-opencode-auth-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let _cleanup = install_opencode_auth_fixture(&root);
        let (mut app, context) = remote_account_fixture(&root);
        let materialization =
            |profile_id: &str, contents_base64: &str| ProviderAccountMaterialization {
                profile: crate::account_profile::ProviderAccountReplicaMetadata {
                    owner_user_id: "owner-a".to_string(),
                    provider: "opencode".to_string(),
                    profile_id: profile_id.to_string(),
                    label: "OpenCode Zen".to_string(),
                    origin: crate::account_profile::ProviderAccountProfileOrigin::Linked,
                    is_default: false,
                },
                files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                    relative_path: "data/opencode/auth.json".to_string(),
                    contents_base64: contents_base64.to_string(),
                }],
                generated_at_ms: 1,
            };

        let profile = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(
                context.clone(),
                materialization(
                    "opencode-valid",
                    "eyJvcGVuY29kZSI6eyJ0eXBlIjoiYXBpIiwia2V5IjoiemVuLXNlY3JldCJ9fQ==",
                ),
            )
            .expect("provider-native auth list should validate transferred credentials");
        assert_eq!(
            profile.auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
        assert!(profile.last_validated_at_ms.is_some());

        let error = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(
                context,
                materialization(
                    "opencode-invalid",
                    "eyJvcGVuY29kZSI6eyJ0eXBlIjoiYXBpIiwia2V5IjoiIn19",
                ),
            )
            .expect_err("malformed transferred credentials must fail closed");
        let message = error.to_string();
        assert!(message.contains("OpenCode Zen"), "{message}");
        assert!(message.contains("not authenticated"), "{message}");
        assert!(!message.contains("eyJvcGVuY29kZSI"), "{message}");
        let stored = app
            .provider_account_profile_registry()
            .get("owner-a", "opencode", "opencode-invalid")
            .expect("failed validation should still persist the observed state");
        assert_eq!(
            stored.auth_state,
            crate::account_profile::ProviderAccountAuthState::NotConfigured
        );
        assert!(stored.last_validated_at_ms.is_some());
    }
}
