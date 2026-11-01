//! MP-08 / MP-10 / MP-11: every inventory grant stays within Cloud's 64-target bound.
use super::{authorization, query_relay_once, with_metadata_token};
use crate::{config::DaemonConfig, error::DaemonError};
use chariox_relay::protocol::{RelayEnvelope, RelayMetadataQuery};

pub(super) async fn query(
    config: &DaemonConfig,
    query: RelayMetadataQuery,
) -> Result<RelayEnvelope, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| invalid("Cloud profile missing"))?;
    let targets: Vec<_> = authorization::visible_targets(profile)
        .await?
        .ok_or_else(|| invalid("kernel inventory missing"))?
        .into_iter()
        .collect();
    let mut combined = None;
    for targets in targets.chunks(64) {
        let token = authorization::cloud_metadata_token_for_targets(
            config,
            Some(targets.iter().cloned().collect()),
        )
        .await?;
        let scoped = with_metadata_token(config, token);
        let response = query_relay_once(&scoped, query.clone()).await?;
        match &response {
            RelayEnvelope::ClientMetadataResponse { error: None, .. } => {}
            _ => return Ok(response), // never report a partial successful inventory
        }
        if let Some(previous) = &mut combined {
            merge(previous, response)?;
        } else {
            combined = Some(response);
        }
    }
    combined.ok_or_else(|| invalid("kernel inventory missing"))
}

fn merge(into: &mut RelayEnvelope, next: RelayEnvelope) -> Result<(), DaemonError> {
    let (
        RelayEnvelope::ClientMetadataResponse {
            machines,
            kernels,
            kernel,
            ..
        },
        RelayEnvelope::ClientMetadataResponse {
            machines: more_machines,
            kernels: more_kernels,
            kernel: more_kernel,
            ..
        },
    ) = (into, next)
    else {
        return Err(invalid("unexpected inventory response"));
    };
    if let Some(more) = more_machines {
        let all = machines.get_or_insert_default();
        for machine in more {
            if let Some(existing) = all
                .iter_mut()
                .find(|existing| existing.machine_id == machine.machine_id)
            {
                existing.kernel_count += machine.kernel_count;
                for provider in machine.available_providers {
                    if !existing.available_providers.contains(&provider) {
                        existing.available_providers.push(provider);
                    }
                }
                for account in machine.provider_accounts {
                    if !existing.provider_accounts.iter().any(|seen| {
                        seen.provider == account.provider
                            && seen.alias == account.alias
                            && seen.email == account.email
                            && seen.account_id == account.account_id
                            && seen.auth_type == account.auth_type
                            && seen.state == account.state
                    }) {
                        existing.provider_accounts.push(account);
                    }
                }
                existing.available_providers.sort();
            } else {
                all.push(machine);
            }
        }
    }
    if let Some(more) = more_kernels {
        let all = kernels.get_or_insert_default();
        for candidate in more {
            if !all
                .iter()
                .any(|existing| existing.kernel_id == candidate.kernel_id)
            {
                all.push(candidate);
            }
        }
    }
    // Preserve the relay's existing first matching alias; batching adds no input admission rule.
    if kernel.is_none() {
        *kernel = more_kernel;
    }

    Ok(())
}

fn invalid(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "relay_metadata_query",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::protocol::{RelayMachinePresence, RelayProviderAccountSummary};

    #[test]
    fn split_lookup_preserves_the_relays_first_match() {
        let response = |id: &str| RelayEnvelope::ClientMetadataResponse {
            request_id: "inventory".into(),
            machines: None,
            kernels: None,
            error: None,
            kernel: Some(
                serde_json::from_value(serde_json::json!({
                    "kernel_id": id, "machine_id": "machine", "public_key": "synthetic-public"
                }))
                .unwrap(),
            ),
        };
        let mut first = response("first");
        merge(&mut first, response("second")).unwrap();
        let RelayEnvelope::ClientMetadataResponse {
            kernel: Some(kernel),
            ..
        } = first
        else {
            panic!("missing kernel")
        };
        assert_eq!(kernel.kernel_id, "first");
    }

    #[test]
    fn split_machine_inventory_preserves_relay_account_identity() {
        let response = |count, organization: &str| RelayEnvelope::ClientMetadataResponse {
            request_id: "inventory".into(),
            machines: Some(vec![RelayMachinePresence {
                machine_id: "machine".into(),
                machine_alias: None,
                kernel_count: count,
                available_providers: vec!["codex".into()],
                provider_accounts: vec![RelayProviderAccountSummary {
                    copyable: false,
                    provider: "codex".into(),
                    state: "linked".into(),
                    account_id: Some("account".into()),
                    alias: None,
                    email: None,
                    auth_type: None,
                    organization_id: None,
                    organization_name: Some(organization.into()),
                    subscription_type: None,
                }],
            }]),
            kernels: None,
            kernel: None,
            error: None,
        };
        let mut first = response(64, "First observation");
        merge(&mut first, response(22, "Later observation")).unwrap();
        let RelayEnvelope::ClientMetadataResponse {
            machines: Some(machines),
            ..
        } = first
        else {
            panic!("missing machines")
        };
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].kernel_count, 86);
        assert_eq!(machines[0].available_providers, ["codex"]);
        assert_eq!(machines[0].provider_accounts.len(), 1);
        assert_eq!(
            machines[0].provider_accounts[0]
                .organization_name
                .as_deref(),
            Some("First observation")
        );
    }
}
