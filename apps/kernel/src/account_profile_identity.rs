//! Claude account identity groups retain each profile's native execution scope.

use super::*;

const CLAUDE_STATE_FILE: &str = ".claude.json";

impl ProviderAccountLocator {
    pub(super) fn claude_account_id(&self) -> Option<String> {
        let Self::Claude {
            claude_config_dir,
            ambient_default,
        } = self
        else {
            return None;
        };
        let state_dir = if *ambient_default == Some(true) {
            claude_config_dir.parent()?
        } else {
            claude_config_dir
        };
        let contents = read_bounded_regular_file_no_follow(
            &state_dir.join(CLAUDE_STATE_FILE),
            MAX_MATERIALIZATION_BYTES,
            CLAUDE_STATE_FILE,
        )
        .ok()??;
        crate::slice_provider_auth::claude_oauth_account_id(&contents)
    }
}

impl StoredProviderAccountProfile {
    fn uses_chariox_login(&self) -> Result<bool, DaemonError> {
        crate::provider::provider_account_credential_registered(
            &self.public.owner_user_id,
            &self.public.provider,
            &self.public.profile_id,
        )
    }

    pub(super) fn observe_claude_account_id(&mut self) -> Result<(), DaemonError> {
        let observed = self.locator.claude_account_id();
        if observed.is_none() && self.provider_account_id.is_none() {
            return Ok(());
        }
        // A registered Chariox login overrides any older native state in this root.
        if self.uses_chariox_login()? {
            self.provider_account_id = None;
            return Ok(());
        }
        if observed.is_some() || !self.materialized_replica {
            self.provider_account_id = observed;
        }
        Ok(())
    }

    fn group_rank(&self) -> (bool, u8) {
        (
            self.public.auth_state == ProviderAccountAuthState::Authenticated,
            self.name_priority(),
        )
    }

    fn name_priority(&self) -> u8 {
        match (self.label_source, self.public.is_default) {
            (Some(ProviderAccountLabelSource::Explicit), true) => 3,
            (Some(ProviderAccountLabelSource::Explicit), false) => 2,
            (_, true) => 1,
            _ => 0,
        }
    }
}

impl RegistryDocument {
    pub(super) fn claude_account(
        &self,
        owner: &str,
        account_id: &str,
    ) -> Option<&StoredProviderAccountProfile> {
        self.claude_bindings(owner, account_id)
            .fold(None, |preferred, candidate| match preferred {
                Some(existing) if existing.name_priority() >= candidate.name_priority() => {
                    Some(existing)
                }
                _ => Some(candidate),
            })
    }

    fn claude_bindings<'a, 'query>(
        &'a self,
        owner: &'query str,
        account_id: &'query str,
    ) -> impl Iterator<Item = &'a StoredProviderAccountProfile> + use<'a, 'query> {
        self.profiles.iter().filter(move |stored| {
            stored.public.owner_user_id == owner
                && stored.public.provider == "claude"
                && stored.provider_account_id.as_deref() == Some(account_id)
        })
    }

    /// The binding a group presents: a usable sign-in beats an unusable one, then the
    /// better-named binding. The display name is chosen separately by `claude_account`.
    fn claude_representative(
        &self,
        owner: &str,
        account_id: &str,
    ) -> Option<&StoredProviderAccountProfile> {
        self.claude_bindings(owner, account_id)
            .fold(None, |preferred, candidate| match preferred {
                Some(existing) if existing.group_rank() >= candidate.group_rank() => Some(existing),
                _ => Some(candidate),
            })
    }

    pub(super) fn visible_profile(
        &self,
        stored: &StoredProviderAccountProfile,
    ) -> Option<ProviderAccountProfile> {
        let Some(account_id) = stored.provider_account_id.as_deref() else {
            return Some(project_usage_freshness(stored.public.clone()));
        };
        let owner = &stored.public.owner_user_id;
        if !std::ptr::eq(stored, self.claude_representative(owner, account_id)?) {
            return None;
        }
        let mut public = stored.public.clone();
        public.label = self.claude_account(owner, account_id)?.public.label.clone();
        public.is_default = self
            .claude_bindings(owner, account_id)
            .any(|binding| binding.public.is_default);
        // Any binding may be the one agents resume on, so the row shows the newest reading.
        if let Some(newest) = self
            .claude_bindings(owner, account_id)
            .map(|binding| &binding.public.usage)
            .filter(|usage| usage.observed_at_ms > public.usage.observed_at_ms)
            .max_by_key(|usage| usage.observed_at_ms)
        {
            public.usage = ProviderAccountUsageSnapshot {
                profile_id: public.profile_id.clone(),
                ..newest.clone()
            };
        }
        Some(project_usage_freshness(public))
    }

    /// The binding that stands for `stored`'s account when it is chosen by name or as the default.
    /// Selection by profile ID stays exact so existing agents keep their own binding.
    pub(super) fn named_binding<'a>(
        &'a self,
        stored: &'a StoredProviderAccountProfile,
    ) -> &'a StoredProviderAccountProfile {
        stored
            .provider_account_id
            .as_deref()
            .and_then(|id| self.claude_representative(&stored.public.owner_user_id, id))
            .unwrap_or(stored)
    }

    /// The binding whose label is the account's displayed name, so a rename lands where it shows.
    pub(super) fn name_holder<'a>(
        &'a self,
        stored: &'a StoredProviderAccountProfile,
    ) -> &'a StoredProviderAccountProfile {
        stored
            .provider_account_id
            .as_deref()
            .and_then(|id| self.claude_account(&stored.public.owner_user_id, id))
            .unwrap_or(stored)
    }

    /// The binding to show for a known account, for callers that return one profile.
    pub(super) fn claude_visible_profile(
        &self,
        owner: &str,
        account_id: &str,
    ) -> Option<ProviderAccountProfile> {
        self.visible_profile(self.claude_representative(owner, account_id)?)
    }

    pub(super) fn remember_claude_merges(&mut self) -> bool {
        let mut changed = false;
        for merge in self.claude_merge_records() {
            if !self.claude_account_merges.contains(&merge) {
                self.claude_account_merges.push(merge);
                changed = true;
            }
        }
        changed
    }

    pub(super) fn backfill_claude_account_ids(&mut self) -> bool {
        let mut changed = false;
        for stored in &mut self.profiles {
            if stored.provider_account_id.is_none() {
                if let Some(id) = stored.locator.claude_account_id() {
                    if stored
                        .uses_chariox_login()
                        .is_ok_and(|registered| !registered)
                    {
                        stored.provider_account_id = Some(id);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn claude_merge_records(&self) -> Vec<ClaudeAccountMerge> {
        self.profiles
            .iter()
            .filter_map(|stored| {
                let account_id = stored.provider_account_id.as_deref()?;
                let owner = &stored.public.owner_user_id;
                if !std::ptr::eq(stored, self.claude_account(owner, account_id)?) {
                    return None;
                }
                let former_names: Vec<_> = self
                    .claude_bindings(owner, account_id)
                    .filter(|binding| !std::ptr::eq(stored, *binding))
                    .map(|binding| binding.public.label.clone())
                    .collect();
                (!former_names.is_empty()).then(|| ClaudeAccountMerge {
                    owner_user_id: owner.clone(),
                    profile_id: stored.public.profile_id.clone(),
                    name: stored.public.label.clone(),
                    former_names,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct ClaudeAccountMerge {
    owner_user_id: String,
    profile_id: String,
    name: String,
    former_names: Vec<String>,
}

pub(super) fn imported_claude_account_id(
    provider: &str,
    profile: &ProviderAccountReplicaMetadata,
) -> Result<Option<String>, DaemonError> {
    profile.provider_account_id.as_deref().map(|id| {
        if provider == "claude" {
            if let Some(id) = crate::slice_provider_auth::normalize_claude_oauth_account_id(id) {
                return Ok(id);
            }
        }
        Err(registry_error("import account profile", "The source machine sent an invalid provider account ID. Refresh the account on the source machine and try again."))
    }).transpose()
}
