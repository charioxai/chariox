//! MP-08 / MP-10 / MP-11: native validation and catalog refresh outside the app lock.
use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountMaterialization, ProviderAccountProfile,
    ProviderAccountProfileRegistry,
};
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::execution_lease::ExecutionLease;
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

pub(crate) struct RemoteProviderAccountProvisioning {
    pub(super) registry: ProviderAccountProfileRegistry,
    pub(super) durable_state: DurableKernelStateStore,
    // MP-08 / MP-10 / MP-11: preserve selected kernel configuration while
    // native validation runs outside the shared app lock.
    pub(super) config: crate::config::DaemonConfig,
    pub(super) lease: ExecutionLease,
    pub(super) context: RemoteProviderAccountSyncContext,
}

impl RemoteProviderAccountProvisioning {
    pub(crate) fn run(
        self,
        mut materialization: ProviderAccountMaterialization,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let lease = &self.lease;
        let context = &self.context;
        let credentialless = materialization.is_credentialless_opencode();
        // MP-10: a login the transfer or an account copy already installed here, from this very account of this home kernel, is the worker's copy: no second one is made.
        // It comes first because the home account may still hold its native login next to the copied setup-token login.
        // Copies and transfers sit under the account authority, not under the lease owner.
        let account_authority = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config,
            &lease.owner_user_id,
        );
        if let Some(transferred) = self.registry.transferred_profile(
            &account_authority,
            &materialization.profile.provider,
            &context.home_kernel_id,
            &materialization.profile.profile_id,
        )? {
            if let Ok(profile) = self.validate_remote_provider_account(transferred) {
                return Ok(profile);
            }
        }

        if crate::provider::canonical_provider_family(&materialization.profile.provider)
            == Some("claude")
            && materialization
                .files
                .iter()
                .any(|file| file.relative_path == ".credentials.json")
        {
            return Err(DaemonError::LocalTransport {
                operation: crate::transport::relay_peer::ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION,
                message: "Claude provider credentials cannot be materialized on a remote worker; a native Claude login has no copyable credential. On your own computer create a setup-token login for it (Provider Accounts: Create setup token, or /provider setup-token claude <account-profile> --run), then on that machine run /provider accounts copy and pick the login, and start the agent again".to_string(),
            });
        }

        self.registry.reconcile_materialized_replica_rollback(
            &lease.owner_user_id,
            &materialization.profile.provider,
            &materialization.profile.profile_id,
        )?;

        let existing_profiles = self.registry.clone().list(
            &lease.owner_user_id,
            Some(&materialization.profile.provider),
        )?;
        if let Some(profile) = existing_profiles
            .iter()
            .find(|profile| profile.profile_id == materialization.profile.profile_id)
            .cloned()
        {
            let legacy_unpinned = self.registry.is_legacy_unpinned_replica(
                &lease.owner_user_id,
                &profile.provider,
                &profile.profile_id,
            )?;
            if legacy_unpinned {
                let registry = self.registry.clone();
                let observed_identity = registry.legacy_replica_root_identity(
                    &lease.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                )?;
                // An old persisted auth state cannot validate the current worker root.
                // Only a fresh native observation may retain or replace that root.
                crate::local::provider_requests::observe_provider_auth_status(
                    &registry,
                    &lease.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                    &self.config,
                )?;
                let observed =
                    registry.get(&lease.owner_user_id, &profile.provider, &profile.profile_id)?;
                if observed.auth_state == ProviderAccountAuthState::Authenticated {
                    let validated = registry.require_authenticated(
                        &lease.owner_user_id,
                        &profile.provider,
                        &profile.profile_id,
                        None,
                        crate::transport::relay_peer::ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION,
                    )?;
                    let observed_identity = observed_identity.ok_or_else(|| DaemonError::LocalTransport {
                    operation: crate::transport::relay_peer::ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION,
                    message: "authenticated legacy account root was absent before provider observation".to_string(),
                })?;
                    registry.pin_validated_legacy_replica(
                        &lease.owner_user_id,
                        &profile.provider,
                        &profile.profile_id,
                        observed_identity,
                    )?;
                    return Ok(validated);
                }
                let retained = registry.retain_failed_legacy_replica(
                    &lease.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                    observed_identity,
                )?;
                if let Some(path) = retained {
                    tracing::warn!(
                        path = %path.display(),
                        "failed legacy provider account retained before fresh materialization"
                    );
                }
            } else if profile.provider == "opencode"
                && profile.auth_state != ProviderAccountAuthState::Authenticated
                && self.registry.opencode_profile_has_no_credentials(
                    &lease.owner_user_id,
                    &profile.profile_id,
                )?
            {
                if credentialless {
                    return Ok(profile);
                }
                // A profile previously installed without credentials can now
                // receive the home login through the usual provisioning transaction.
            } else {
                return self.validate_remote_provider_account(profile);
            }
        }
        materialization.profile.origin =
            crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated;
        let (profile, previous_default_profile_id) = self
            .registry
            .materialize_replica_with_rollback_state(&lease.owner_user_id, &materialization)?;
        let profile = match (|| {
            let profile = if credentialless {
                // Model selection runs on this worker after provisioning, using
                // what OpenCode actually offers in the isolated profile.
                self.registry.update_observation(
                    &profile.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                    ProviderAccountAuthState::NotConfigured,
                    None,
                    None,
                    None,
                    None,
                )?
            } else {
                self.validate_remote_provider_account(profile)?
            };
            if profile.provider == "opencode" {
                let environment = self.registry.resolve_environment(
                    &profile.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                )?;
                crate::local::provider_requests::prepare_opencode_model_cache(
                    &environment,
                    &profile.services,
                )?;
            }
            Ok(profile)
        })() {
            Ok(profile) => profile,
            Err(error) => {
                if let Err(rollback_error) = self
                    .registry
                    .rollback_materialized_replica_restoring_default(
                        &lease.owner_user_id,
                        &materialization.profile.provider,
                        &materialization.profile.profile_id,
                        previous_default_profile_id.as_deref(),
                    )
                {
                    let message = match &error {
                        DaemonError::ProviderProfileRefused { reason, .. } => {
                            crate::error::external_request_failure_message(reason.code())
                                .to_string()
                        }
                        _ => error.to_string(),
                    };
                    return Err(DaemonError::LocalTransport {
                        operation: crate::transport::relay_peer::ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION,
                        message: format!(
                            "{message}; additionally failed to roll back the new provider-account replica: {rollback_error}"
                        ),
                    });
                }
                return Err(error);
            }
        };
        self.registry.mark_materialized_replica_validated(
            &lease.owner_user_id,
            &materialization.profile.provider,
            &materialization.profile.profile_id,
        )?;
        self.registry.record_credential_copy(
            &lease.owner_user_id,
            &materialization,
            &context.home_kernel_id,
        )?;
        self.durable_state.append_event(
            "provider_account.materialized",
            Some(lease.id.clone()),
            serde_json::json!({
                "owner_user_id": profile.owner_user_id,
                "provider": profile.provider,
                "profile_id": profile.profile_id,
                "source_home_kernel_id": context.home_kernel_id,
            }),
        )?;
        Ok(profile)
    }

    fn validate_remote_provider_account(
        &self,
        profile: ProviderAccountProfile,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        if !remote_provider_account_requires_auth_validation(&profile.provider, profile.auth_state)
        {
            return Ok(profile);
        }
        let registry = self.registry.clone();
        crate::local::provider_requests::observe_provider_auth_status(
            &registry,
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
            &self.config,
        )?;
        registry.require_authenticated(
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
            None,
            crate::transport::relay_peer::ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION,
        )
    }
}

pub(super) fn remote_provider_account_requires_auth_validation(
    provider: &str,
    auth_state: ProviderAccountAuthState,
) -> bool {
    // Claude refresh credentials never cross this boundary. Its official CLI
    // is authenticated at launch through the existing vaulted setup-token
    // path, so preserve that separate handoff contract.
    crate::provider::canonical_provider_family(provider) != Some("claude")
        && auth_state != ProviderAccountAuthState::Authenticated
}
