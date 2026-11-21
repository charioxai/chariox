//! Public account references contain identity and status, never native profile locations.
use super::*;
use crate::local::ProviderUsageAccount;

impl ProviderAccountProfileRegistry {
    pub(crate) fn usage_accounts(
        &self,
        owner_user_id: &str,
        machine_id: &str,
    ) -> Result<Vec<ProviderUsageAccount>, DaemonError> {
        let document = self.read_document()?;
        Ok(document
            .profiles
            .iter()
            .filter_map(|stored| {
                let profile = document.visible_profile(stored)?;
                if profile.owner_user_id != owner_user_id {
                    return None;
                }
                // Startup reserves native defaults before anyone has linked a login.
                // Those empty placeholders are not provider accounts.
                if profile.origin == ProviderAccountProfileOrigin::Default
                    && matches!(
                        profile.auth_state,
                        ProviderAccountAuthState::Unknown | ProviderAccountAuthState::NotConfigured
                    )
                    && profile.usage.meters.is_empty()
                {
                    return None;
                }
                let mut identity = Sha256::new();
                identity.update(b"chariox.provider-account.v1\0");
                identity.update(profile.provider.as_bytes());
                identity.update([0]);
                if let Some(account_id) = &stored.provider_account_id {
                    identity.update(b"provider-account-id\0");
                    identity.update(account_id.as_bytes());
                } else if identity_label_base(profile.identity_summary.as_deref()).is_some() {
                    identity.update(b"provider-identity\0");
                    identity.update(
                        profile
                            .identity_summary
                            .as_ref()
                            .unwrap()
                            .trim()
                            .to_lowercase()
                            .as_bytes(),
                    );
                } else {
                    // OpenCode and API-key accounts need not expose a personal identity.
                    // Only the same native store on the same machine is known to be one account.
                    identity.update(b"native-scope\0");
                    identity.update(machine_id.as_bytes());
                    for path in stored.locator.roots() {
                        identity.update([0]);
                        let scope =
                            if matches!(stored.locator, ProviderAccountLocator::Claude { .. }) {
                                path.to_path_buf()
                            } else {
                                path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
                            };
                        identity.update(scope.to_string_lossy().as_bytes());
                    }
                    if let ProviderAccountLocator::Claude {
                        ambient_default, ..
                    } = &stored.locator
                    {
                        identity.update(format!("{ambient_default:?}").as_bytes());
                    }
                }
                let public = profile;
                Some(ProviderUsageAccount {
                    account_id: format!("{:x}", identity.finalize()),
                    provider: public.provider,
                    profile_id: public.profile_id,
                    label: public.label,
                    identity_summary: public.identity_summary,
                    auth_state: public.auth_state,
                    services: public.services,
                    usage: public.usage,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_identity_follows_provider_identity_and_never_display_names() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("usage-account-identity");
        let a = ProviderAccountProfileRegistry::open(root.path().join("a.json")).unwrap();
        let b = ProviderAccountProfileRegistry::open(root.path().join("b.json")).unwrap();
        for (registry, email) in [(&a, "Work@Example.test"), (&b, "work@example.test")] {
            let profile = registry.create_managed("local", "claude", "Work").unwrap();
            registry
                .update_observation(
                    "local",
                    "claude",
                    &profile.profile_id,
                    ProviderAccountAuthState::Authenticated,
                    Some(email.into()),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }
        let first = a.usage_accounts("local", "machine-a").unwrap();
        let second = b.usage_accounts("local", "machine-b").unwrap();
        assert_eq!(first[0].account_id, second[0].account_id);
        assert!(a
            .usage_accounts("other-owner", "machine-a")
            .unwrap()
            .is_empty());
        let renamed = b
            .rename("local", "claude", &second[0].profile_id, "Personal")
            .unwrap();
        assert_eq!(
            b.usage_accounts("local", "machine-b").unwrap()[0].account_id,
            first[0].account_id
        );
        b.update_observation(
            "local",
            "claude",
            &renamed.profile_id,
            ProviderAccountAuthState::Authenticated,
            Some("other@example.test".into()),
            None,
            None,
            None,
        )
        .unwrap();
        assert_ne!(
            b.usage_accounts("local", "machine-b").unwrap()[0].account_id,
            first[0].account_id
        );
    }

    #[test]
    fn usage_identity_only_merges_anonymous_profiles_with_the_same_native_scope() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("usage-native-scope");
        let a = ProviderAccountProfileRegistry::open(root.path().join("a.json")).unwrap();
        let b = ProviderAccountProfileRegistry::open(root.path().join("b.json")).unwrap();
        let scope = root.path().join("opencode");
        fs::create_dir_all(&scope).unwrap();
        set_private_dir_permissions(&scope).unwrap();
        for registry in [&a, &b] {
            registry
                .link_existing("local", "opencode", "Work", &scope)
                .unwrap();
        }
        let first = a.usage_accounts("local", "same-machine").unwrap();
        let second = b.usage_accounts("local", "same-machine").unwrap();
        assert_eq!(first[0].account_id, second[0].account_id);
        assert_ne!(
            first[0].account_id,
            b.usage_accounts("local", "another-machine").unwrap()[0].account_id
        );
        let encoded = serde_json::to_string(&first).unwrap();
        assert!(!encoded.contains(scope.to_str().unwrap()));
    }
}

#[cfg(test)]
mod claude_account_identity_tests {
    use super::*;

    fn native_login(root: &Path, installation: &str, account: &str) {
        fs::create_dir_all(root).unwrap();
        set_private_dir_permissions(root).unwrap();
        fs::write(
            root.join(".claude.json"),
            serde_json::to_vec(&serde_json::json!({
                "userID": installation,
                "oauthAccount": { "accountUuid": account }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn second_claude_link_names_the_existing_account() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-account-duplicate-link");
        let registry =
            ProviderAccountProfileRegistry::open(root.path().join("accounts.json")).unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        let account = "879d9f26-dd4a-40f5-9aa1-e9ebc8ffc16a";
        native_login(&first, "installation-a", account);
        native_login(&second, "installation-b", account);
        registry
            .link_existing("local", "claude", "Work", &first)
            .unwrap();
        let refusal = registry
            .link_existing("local", "claude", "Personal", &second)
            .unwrap_err();
        assert!(refusal.to_string().contains("same Claude account"));
        assert!(refusal.to_string().contains("Work"));
        assert_eq!(registry.list("local", Some("claude")).unwrap().len(), 1);
        registry
            .link_existing("other-owner", "claude", "Personal", &second)
            .unwrap();
    }

    #[test]
    fn claude_account_upgrade_keeps_the_chosen_name_and_both_execution_roots() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-account-group-upgrade");
        let path = root.path().join("accounts.json");
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        let third = root.path().join("third");
        let account = "879d9f26-dd4a-40f5-9aa1-e9ebc8ffc16a";
        let other_account = "3923d56f-cd51-4bb3-a129-ed90956dc8f3";
        // Legacy registrations were possible before the identity was learned.
        for dir in [&first, &second, &third] {
            fs::create_dir_all(dir).unwrap();
            set_private_dir_permissions(dir).unwrap();
        }
        let first_profile = registry
            .link_existing("local", "claude", "Work", &first)
            .unwrap();
        let second_profile = registry
            .link_existing("local", "claude", "Personal", &second)
            .unwrap();
        let third_profile = registry
            .link_existing("local", "claude", "Another", &third)
            .unwrap();
        for profile in [&first_profile, &second_profile] {
            registry
                .update_observation(
                    "local",
                    "claude",
                    &profile.profile_id,
                    ProviderAccountAuthState::Authenticated,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
        }
        registry
            .set_default("local", "claude", &second_profile.profile_id)
            .unwrap();
        drop(registry);
        native_login(&first, "installation-a", account);
        native_login(&second, "installation-b", account);
        native_login(&third, "installation-c", other_account);

        for _ in 0..2 {
            let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
            let listed = registry.list("local", Some("claude")).unwrap();
            assert_eq!(listed.len(), 2);
            assert_eq!(
                listed.iter().find(|p| p.is_default).unwrap().label,
                "Personal"
            );
            let merges =
                serde_json::to_value(&registry.read_document().unwrap().claude_account_merges)
                    .unwrap();
            assert_eq!(
                merges,
                serde_json::json!([{
                    "owner_user_id": "local",
                    "profile_id": second_profile.profile_id,
                    "name": "Personal",
                    "former_names": ["Work"]
                }])
            );
            let usage = registry.usage_accounts("local", "machine-a").unwrap();
            assert_eq!(usage.len(), 2);
            assert!(usage.iter().any(|row| row.label == "Personal"));
            for (profile, dir) in [(&first_profile, &first), (&second_profile, &second)] {
                registry
                    .require_authenticated(
                        "local",
                        "claude",
                        &profile.profile_id,
                        None,
                        "resume drill",
                    )
                    .unwrap();
                let env = registry
                    .resolve_environment("local", "claude", &profile.profile_id)
                    .unwrap();
                assert_eq!(
                    env.get("CLAUDE_CONFIG_DIR").unwrap(),
                    &dir.display().to_string()
                );
            }
        }
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        registry
            .set_default("local", "claude", &third_profile.profile_id)
            .unwrap();
        let listed = registry.list("local", Some("claude")).unwrap();
        assert!(listed.iter().any(|p| p.label == "Work"));
        assert!(listed.iter().all(|p| p.label != "Personal"));
    }

    #[test]
    fn native_claude_transfer_carries_only_the_account_id_as_identity_metadata() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-account-transfer-identity");
        let source = ProviderAccountProfileRegistry::open(root.path().join("source.json")).unwrap();
        let target = ProviderAccountProfileRegistry::open(root.path().join("target.json")).unwrap();
        let native = root.path().join("native");
        let account = "879d9f26-dd4a-40f5-9aa1-e9ebc8ffc16a";
        native_login(&native, "installation-a", account);
        atomic_write_private(
            &native.join(".credentials.json"),
            br#"{"claudeAiOauth":{"refreshToken":"synthetic-refresh"}}"#,
        )
        .unwrap();
        let profile = source
            .link_existing("local", "claude", "Work", &native)
            .unwrap();
        source
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let materialization = source
            .export_materialization("local", "claude", &profile.profile_id)
            .unwrap();
        assert_eq!(
            materialization.profile.provider_account_id.as_deref(),
            Some(account)
        );
        assert!(!serde_json::to_string(&materialization.profile)
            .unwrap()
            .contains("installation-a"));
        assert!(materialization
            .files
            .iter()
            .all(|file| file.relative_path != ".claude.json"));
        let imported = target
            .materialize_replica("local", &materialization)
            .unwrap();
        assert_eq!(imported.profile_id, profile.profile_id);
        let second = root.path().join("same-account");
        native_login(&second, "installation-b", account);
        let refusal = target
            .link_existing("local", "claude", "Duplicate", &second)
            .unwrap_err();
        assert!(refusal.to_string().contains("Work"));
    }
}
