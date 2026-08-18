use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunState;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_account_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    let runtime_state = runtime_state.clone();
    tokio::task::spawn_blocking(move || match request {
        LocalDaemonRequest::ListProviderAccountProfiles(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfilesListed {
                profiles: registry.list(&owner_user_id, request.provider.as_deref())?,
            })
        }
        LocalDaemonRequest::GetProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.get(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::CreateProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.create_managed(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::LinkProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.link_existing(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                    &PathBuf::from(request.path),
                )?,
            })
        }
        LocalDaemonRequest::RenameProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.rename(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::SetDefaultProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.set_default(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::RefreshProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile:
                    crate::local::provider_requests::refresh_provider_account_profile_response(
                        &registry,
                        &owner_user_id,
                        &request.provider,
                        &request.account_profile,
                    )?,
            })
        }
        LocalDaemonRequest::RemoveProviderAccountProfile(request) => {
            ensure_profile_idle(
                &runtime_state,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            Ok(LocalDaemonResponse::ProviderAccountProfileRemoved {
                profile: registry.remove_registration(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::DeleteProviderAccountProfileData(request) => {
            ensure_profile_idle(
                &runtime_state,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            Ok(LocalDaemonResponse::ProviderAccountProfileDataDeleted {
                profile: registry.delete_managed_profile_data(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.confirmation_profile_id,
                )?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider account request",
            message: "unsupported provider account request".to_string(),
        }),
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "provider account request",
        message: error.to_string(),
    })?
}

pub(crate) fn ensure_profile_idle(
    runtime_state: &KernelRuntimeState,
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<(), DaemonError> {
    let profile = registry.get(owner_user_id, provider, profile_id)?;
    let active = runtime_state
        .provider_runs_for_external_session_attachment()
        .into_iter()
        .any(|run| {
            run.owner_user_id() == owner_user_id
                && crate::provider::canonical_provider_family(run.provider())
                    == crate::provider::canonical_provider_family(provider)
                && run.account_profile() == profile.profile_id
                && run.state() != ProviderRunState::Ended
        });
    if active {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` has an active provider run; end the run before removing or deleting it",
                profile.profile_id
            ),
        });
    }
    Ok(())
}
