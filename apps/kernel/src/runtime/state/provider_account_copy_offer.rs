//! Recipient admission and consent for independent account copies.
use super::*;
use crate::local::CopyProviderAccountFromKernelRequest;
use crate::provider_account_copy::{
    failure, ProviderAccountCopyBinding, ProviderAccountCopyOffer, ProviderAccountCopyReceipt,
    PROVIDER_ACCOUNT_COPY_PULL_CAPABILITY,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};
use sha2::{Digest, Sha256};

struct CopyConsent {
    state: KernelRuntimeState,
    id: String,
}
impl Drop for CopyConsent {
    fn drop(&mut self) {
        let _ = self.state.owned.timeout_runtime_interaction("", &self.id);
    }
}

fn enrolled_owner(
    config: &crate::config::DaemonConfig,
    caller: &str,
) -> Result<String, DaemonError> {
    let enrollment = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| failure("provider_account_copy_unauthorized"))?;
    if ![
        crate::session::DEFAULT_LOCAL_USER_ID,
        enrollment.user_id.as_str(),
    ]
    .contains(&caller)
        || enrollment.user_id.is_empty()
        || enrollment.realm_id.is_empty()
        || enrollment.machine_id.as_deref() != Some(config.host_machine_id.as_str())
        || enrollment.kernel_id.as_deref() != Some(config.daemon_id.as_str())
        || enrollment.kernel_public_key_thumbprint.as_deref()
            != Some(
                crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
                    .as_str(),
            )
    {
        return Err(failure("provider_account_copy_unauthorized"));
    }
    Ok(enrollment.user_id.clone())
}

impl KernelRuntimeState {
    pub(crate) async fn list_provider_account_copy_offers(
        &self,
        caller: &str,
    ) -> Result<Vec<ProviderAccountCopyOffer>, DaemonError> {
        let mut offers = self.discover_provider_account_copy_offers(caller).await?;
        let owner = self.provider_account_authority_owner_user_id(caller);
        let installed = self
            .provider_account_profile_registry()
            .list(&owner, Some("claude"))?
            .into_iter()
            .map(|profile| profile.profile_id)
            .collect::<std::collections::BTreeSet<_>>();
        offers.retain(|offer| {
            !installed.contains(&crate::provider_account_copy::copy_profile_id(
                &offer.source_kernel_id,
                &offer.source_key_thumbprint,
                &offer.provider,
                &offer.account_profile,
                &offer.target_kernel_id,
            ))
        });
        Ok(offers)
    }

    async fn discover_provider_account_copy_offers(
        &self,
        caller: &str,
    ) -> Result<Vec<ProviderAccountCopyOffer>, DaemonError> {
        let config = self.config_snapshot().await;
        enrolled_owner(&config, caller)?;
        let peers = crate::managed_context::owner_authority::account_copy_peers(&config).await?;
        let discovery = crate::transport::relay_discovery::metadata_discovery_config(&config)
            .await
            .map_err(|_| failure("provider_account_copy_unavailable"))?;
        let machines = crate::transport::relay_discovery::list_live_machines(&discovery)
            .await
            .map_err(|_| failure("provider_account_copy_unavailable"))?;
        let mut offers = Vec::new();
        for machine in machines {
            if !peers.iter().any(|peer| {
                peer.machine_id == machine.machine_id && peer.kernel_id != config.daemon_id
            }) {
                continue;
            }
            let kernels = crate::transport::relay_discovery::list_live_kernels_for_machine(
                &discovery,
                &machine.machine_id,
            )
            .await
            .map_err(|_| failure("provider_account_copy_unavailable"))?;
            for kernel in kernels {
                let Some(peer) = peers.iter().find(|peer| {
                    peer.kernel_id == kernel.kernel_id
                        && peer.relay_public_key == kernel.public_key
                        && peer.relay_realm_id
                            == config.cloud_relay.as_ref().expect("enrolled").realm_id
                        && peer.key_thumbprint
                            == crate::runtime::terminal_pairings::public_key_thumbprint(
                                &kernel.public_key,
                            )
                }) else {
                    continue;
                };
                if kernel.kernel_id == config.daemon_id
                    || !kernel
                        .capabilities
                        .iter()
                        .any(|c| c == PROVIDER_ACCOUNT_COPY_PULL_CAPABILITY)
                {
                    continue;
                }
                for profile in kernel.provider_accounts {
                    let Some(account_profile) = profile
                        .account_id
                        .filter(|id| crate::provider_account_copy::valid_id(id))
                    else {
                        continue;
                    };
                    if profile.provider != "claude"
                        || profile.state != "authenticated"
                        || !profile.copyable
                    {
                        continue;
                    }
                    offers.push(ProviderAccountCopyOffer {
                        source_kernel_id: kernel.kernel_id.clone(),
                        source_machine_id: kernel.machine_id.clone(),
                        source_label: kernel
                            .kernel_alias
                            .clone()
                            .or(kernel.machine_alias.clone())
                            .unwrap_or_else(|| kernel.kernel_id.clone()),
                        source_key_thumbprint: peer.key_thumbprint.clone(),
                        target_kernel_id: config.daemon_id.clone(),
                        provider: profile.provider,
                        label: profile.alias.unwrap_or_else(|| account_profile.clone()),
                        account_profile,
                    });
                }
            }
        }
        offers.sort_by(|a, b| {
            (
                &a.source_label,
                &a.label,
                &a.source_kernel_id,
                &a.account_profile,
            )
                .cmp(&(
                    &b.source_label,
                    &b.label,
                    &b.source_kernel_id,
                    &b.account_profile,
                ))
        });
        Ok(offers)
    }

    /// The owner's own client (`command.is_terminal_caller()`) asked for this
    /// copy by choosing the offer, so that choice is the consent. Any other
    /// caller (an agent, a leased or remote run, a Metaagent) still needs the
    /// kernel decision. The source kernel's release rules do not depend on it.
    pub(crate) async fn copy_provider_account_from_kernel(
        &self,
        command: &KernelCommand,
        request: CopyProviderAccountFromKernelRequest,
    ) -> Result<ProviderAccountCopyReceipt, DaemonError> {
        let caller = &command_caller_user_id(command);
        let config = self.config_snapshot().await;
        let owner = enrolled_owner(&config, caller)?;
        let binding = ProviderAccountCopyBinding {
            operation_id: request.operation_id,
            provider: request.provider,
            source_profile_id: request.account_profile,
            source_kernel_id: request.source_kernel_id,
            source_key_thumbprint: request.source_key_thumbprint,
            target_kernel_id: config.daemon_id.clone(),
            target_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &config.relay_public_key,
            ),
            replace: request.replace,
        };
        binding.validate()?;
        let offer = self
            .discover_provider_account_copy_offers(caller)
            .await?
            .into_iter()
            .find(|offer| {
                offer.source_kernel_id == binding.source_kernel_id
                    && offer.source_key_thumbprint == binding.source_key_thumbprint
                    && offer.provider == binding.provider
                    && offer.account_profile == binding.source_profile_id
            })
            .ok_or_else(|| failure("provider_account_copy_profile"))?;
        if !command.is_terminal_caller() {
            self.await_copy_consent(&owner, &binding, &offer).await?;
        }
        let current = self.config_snapshot().await;
        if enrolled_owner(&current, caller)? != owner
            || current.daemon_id != config.daemon_id
            || current.relay_public_key != config.relay_public_key
            || current.cloud_relay.as_ref().map(|p| &p.realm_id)
                != config.cloud_relay.as_ref().map(|p| &p.realm_id)
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        let discovery = crate::transport::relay_discovery::metadata_discovery_config(&current)
            .await
            .map_err(|_| failure("provider_account_copy_unavailable"))?;
        let source = crate::transport::relay_discovery::get_live_kernel(
            &discovery,
            &binding.source_kernel_id,
        )
        .await
        .map_err(|_| failure("provider_account_copy_unavailable"))?;
        if crate::runtime::terminal_pairings::public_key_thumbprint(&source.public_key)
            != binding.source_key_thumbprint
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        let response = crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
            &current, &self.owned.relay_state,
            ClientTarget { daemon_id: Some(source.kernel_id), daemon_alias: None }, &source.public_key,
            RelayPeerRequest::RequestProviderAccountCopy { request: crate::local::CopyProviderAccountToKernelRequest {
                provider: binding.provider.clone(), account_profile: binding.source_profile_id.clone(),
                operation_id: binding.operation_id.clone(), target_kernel_id: binding.target_kernel_id.clone(),
                target_key_thumbprint: binding.target_key_thumbprint.clone(), replace: binding.replace,
            }}, Duration::from_secs(140),
        ).await.map_err(|_| failure("provider_account_copy_unavailable"))?;
        match response {
            RelayPeerResponse::ProviderAccountCopyInstalled { receipt }
                if receipt == ProviderAccountCopyReceipt::installed(&binding) =>
            {
                Ok(receipt)
            }
            RelayPeerResponse::ProviderAccountCopyFailed { code } => Err(copy_peer_failure(&code)),
            _ => Err(failure("provider_account_copy_unsupported")),
        }
    }

    async fn await_copy_consent(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        offer: &ProviderAccountCopyOffer,
    ) -> Result<(), DaemonError> {
        let id = format!(
            "provider-account-copy:{:x}",
            Sha256::digest(serde_json::to_vec(&binding).map_err(|_| failure("invalid_request"))?)
        );
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Copy login to this machine",
            format!(
                "Copy {} from {} to this machine? Each machine keeps an independent login.{}",
                offer.label,
                offer.source_label,
                if binding.replace {
                    " This replaces the previously copied login."
                } else {
                    ""
                }
            ),
            vec![
                RuntimeInteractionChoice::new(
                    "copy",
                    "Copy login to this machine",
                    "copy",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "cancel",
                    "Cancel",
                    "cancel",
                    Some(RuntimeInteractionChoiceStyle::Secondary),
                ),
            ],
        );
        let receiver = self
            .create_kernel_operation_interaction("", owner, interaction)
            .await?;
        let _consent = CopyConsent {
            state: self.clone(),
            id,
        };
        if !matches!(tokio::time::timeout(Duration::from_secs(300), receiver).await,
            Ok(Ok(answer)) if answer.status == "answered" && answer.choice_id.as_deref() == Some("copy"))
        {
            return Err(failure("provider_account_copy_cancelled"));
        }
        Ok(())
    }

    pub(crate) async fn send_requested_provider_account_copy(
        &self,
        target_kernel: &str,
        target_key: &str,
        identity: &chariox_relay::protocol::RelayCallerIdentity,
        request: crate::local::CopyProviderAccountToKernelRequest,
    ) -> Result<ProviderAccountCopyReceipt, DaemonError> {
        let config = self.config_snapshot().await;
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| failure("provider_account_copy_unauthorized"))?;
        if !matches!(
            identity.subject_kind,
            chariox_relay::auth::RelaySubjectKind::Kernel
                | chariox_relay::auth::RelaySubjectKind::Machine
        ) || (identity.subject_kind == chariox_relay::auth::RelaySubjectKind::Kernel
            && chariox_relay::protocol::canonical_peer_daemon_id(&identity.subject)
                != Some(target_kernel))
            || identity.user_id.as_deref() != Some(profile.user_id.as_str())
            || identity.realm_id != profile.realm_id
            || request.target_kernel_id != target_kernel
            || request.target_key_thumbprint
                != crate::runtime::terminal_pairings::public_key_thumbprint(target_key)
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        self.copy_provider_account_to_kernel(&profile.user_id, request)
            .await
    }
}

fn copy_peer_failure(code: &str) -> DaemonError {
    // Accept only the existing constant public error vocabulary, never peer text.
    match code {
        "provider_account_copy_unauthorized" => failure("provider_account_copy_unauthorized"),
        "provider_account_copy_profile" => failure("provider_account_copy_profile"),
        "provider_account_copy_vault_locked" => failure("provider_account_copy_vault_locked"),
        "provider_account_copy_conflict" => failure("provider_account_copy_conflict"),
        "provider_account_copy_rejected" => failure("provider_account_copy_rejected"),
        "provider_account_copy_verification_failed" => {
            failure("provider_account_copy_verification_failed")
        }
        "provider_account_copy_unavailable" => failure("provider_account_copy_unavailable"),
        _ => failure("provider_account_copy_install_failed"),
    }
}
