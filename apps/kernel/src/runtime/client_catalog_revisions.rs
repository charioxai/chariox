//! MP-08 / MP-10 / MP-11: small public revisions through the ordinary catalog service.
use crate::error::DaemonError;
use crate::local::{ClientCatalogRevisions, GetProviderCatalogRequest, LocalDaemonResponse};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(super) async fn catalog_revisions(
    providers: &ProviderCatalogProjectionStore,
    config: &DaemonConfigProjectionStore,
    accounts: &crate::account_profile::ProviderAccountProfileRegistry,
    owner: &str,
    request: GetProviderCatalogRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner =
        crate::account_profile::provider_account_authority_owner_user_id(&config.snapshot(), owner);
    let profiles = accounts.list(&owner, request.provider.as_deref())?;
    let catalog = super::provider_catalog_control::load_catalog(
        providers, config, accounts, &owner, request, false,
    )
    .await?;
    let commands =
        super::provider_catalog_control::execute_get_provider_command_catalogs_request()?;
    let terminal = super::terminal_command_catalog::terminal_command_catalog()?;
    Ok(LocalDaemonResponse::ClientCatalogRevisions {
        revisions: ClientCatalogRevisions {
            provider_accounts: revision(&profiles)?,
            provider_catalog: revision(&catalog)?,
            provider_commands: revision(&commands)?,
            terminal_commands: terminal.revision,
        },
    })
}

fn revision(value: &impl Serialize) -> Result<String, DaemonError> {
    serde_json::to_vec(value)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "catalog revision",
            message: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp10_mp11_catalog_revisions_are_content_hashes() {
        let value = serde_json::json!({"models": {"one": {"name":"one"}}});
        let same = serde_json::json!({"models": {"one": {"name":"one"}}});
        let changed = serde_json::json!({"models": {"one": {"name":"updated"}}});
        assert_eq!(revision(&value).unwrap(), revision(&same).unwrap());
        assert_ne!(revision(&value).unwrap(), revision(&changed).unwrap());
        assert_eq!(revision(&value).unwrap().len(), 64);
    }
}
