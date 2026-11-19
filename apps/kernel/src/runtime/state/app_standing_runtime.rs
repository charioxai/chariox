//! The owner decides, once, that an action class of an App needs no card in one repository.
//! Only the owner can: there is no agent tool and no App import for it, and the gate that
//! reads these (`invoke_declared`) can only skip a card the App's signed declaration lets it.
use super::KernelRuntimeState;
use crate::durable_state::app_standing::{StandingCommand, StandingPermission};
use crate::local::{
    AppRequestErrorCode, AppStandingSummary, LocalDaemonRequest, LocalDaemonResponse,
};

/// `owner/name`: what the declaring action's input names, compared as lowercase text.
pub(crate) fn valid_repo(repo: &str) -> bool {
    repo.len() <= 200
        && repo.bytes().all(|b| b.is_ascii_graphic())
        && repo.split('/').count() == 2
        && repo.split('/').all(|part| !part.is_empty())
}

impl KernelRuntimeState {
    pub(super) async fn app_standing_request(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let command = match request {
            LocalDaemonRequest::GrantAppStanding(request) => {
                let now = crate::session::unix_epoch_ms();
                if !valid_repo(&request.repo)
                    || request.expires_at_ms.is_some_and(|expires| expires <= now)
                {
                    return Err(AppRequestErrorCode::InvalidRequest);
                }
                // A class no action of this release declares has nothing to cover.
                let store = self.owned.durable_state_store.clone();
                let (catalog_owner, catalog_installation) = (owner.clone(), installation.clone());
                let covered = tokio::task::spawn_blocking(move || {
                    store
                        .active_app_event_catalog(&catalog_owner, &catalog_installation)
                        .map(|catalog| {
                            catalog.app_catalog().ui_actions().iter().any(|action| {
                                action
                                    .standing
                                    .as_ref()
                                    .is_some_and(|standing| standing.class == request.class)
                            })
                        })
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|_| AppRequestErrorCode::NotFound)?;
                if !covered {
                    return Err(AppRequestErrorCode::InvalidRequest);
                }
                Some(StandingCommand::Grant {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    permission: StandingPermission {
                        id: format!("standing-{:016x}", rand::random::<u64>()),
                        account: request.account,
                        repo: request.repo.to_ascii_lowercase(),
                        class: request.class,
                        created_at_ms: now,
                        expires_at_ms: request.expires_at_ms,
                        uses: 0,
                        last_used_ms: None,
                    },
                })
            }
            LocalDaemonRequest::RevokeAppStanding(request) => Some(StandingCommand::Revoke {
                owner: owner.clone(),
                installation: installation.clone(),
                id: request.standing_id,
            }),
            LocalDaemonRequest::ListAppStanding(_) => None,
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        };
        let store = self.owned.durable_state_store.clone();
        let permit = self.app_control().try_admit()?;
        let standing = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(command) = command {
                store.app_standing_command(command)?;
            }
            store
                .app_standing(&owner, &installation)
                .map(|list| (installation, list))
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|code| match code {
            "NOT_FOUND" => AppRequestErrorCode::NotFound,
            "LIMIT_EXCEEDED" => AppRequestErrorCode::LimitExceeded,
            _ => AppRequestErrorCode::StorageUnavailable,
        })?;
        Ok(LocalDaemonResponse::AppStanding {
            installation_id: standing.0,
            standing: standing
                .1
                .into_iter()
                .map(|permission| AppStandingSummary {
                    standing_id: permission.id,
                    account: permission.account,
                    repo: permission.repo,
                    class: permission.class,
                    created_at_ms: permission.created_at_ms,
                    expires_at_ms: permission.expires_at_ms,
                    uses: permission.uses,
                    last_used_ms: permission.last_used_ms,
                })
                .collect(),
        })
    }
}
