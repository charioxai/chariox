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
                let profile = &stored.public;
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
                if identity_label_base(profile.identity_summary.as_deref()).is_some() {
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
                let public = project_usage_freshness(profile.clone());
                Some(ProviderUsageAccount {
                    identity_summary: profile.identity_summary.clone(),
                    account_id: format!("{:x}", identity.finalize()),
                    provider: public.provider,
                    profile_id: public.profile_id,
                    label: public.label,
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
