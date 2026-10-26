//! DEV/CLI observation uses the existing Room App binding and verified release.
use super::KernelRuntimeState;
use crate::{
    error::DaemonError,
    local::{InspectAppViewRequest, LocalDaemonResponse},
    runtime::{app_views::AppViewBinding, browser_controller_app_view::BrowserAppViewRequest},
};
use chariox_app_runtime::view_catalog::{ViewCatalog, ViewObservation};

use super::app_view_runtime::open_error as error;

impl KernelRuntimeState {
    pub(super) async fn inspect_app_view(
        &self,
        owner: String,
        request: &InspectAppViewRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let target = match &request.target_id {
            Some(target) if !target.is_empty() && target.len() <= 256 => target.clone(),
            Some(_) => {
                return Err(error(
                    "Invalid App target; open the App view and retry /app actions",
                ))
            }
            None => {
                let tab = self
                    .room_environment_snapshot(&request.session_id)
                    .map_err(|_| {
                        error("The Room environment is unavailable; start it before /app actions")
                    })?
                    .focused_tab_id
                    .ok_or_else(|| error("Focus an open App view before using /app actions"))?;
                self.room_environment_controller_tab_binding(&request.session_id, &tab)
                    .map_err(|_| error("The focused Tab is unavailable; reopen the App view"))?
                    .runtime_target_id
            }
        };
        let views = self.app_control().views().clone();
        let (binding, reconnecting) = views
            .binding_state(&request.session_id, &target)
            .ok_or_else(|| error("This Tab is not an open App view; use /app open first"))?;
        if reconnecting || binding.owner != owner {
            return Err(error(
                "The App view is unavailable or reloading; retry after opening it",
            ));
        }
        let development = self
            .check_inspected_release(&request.session_id, &binding)
            .await?;
        let observation: ViewObservation = self
            .app_view_command(
                &request.session_id,
                BrowserAppViewRequest::Inspect {
                    target_id: target.clone(),
                },
            )
            .await?;
        if views.binding_state(&request.session_id, &target) != Some((binding.clone(), false)) {
            return Err(error(
                "The App view changed during inspection; retry /app actions",
            ));
        }
        self.check_inspected_release(&request.session_id, &binding)
            .await?;
        // The active-release service currently admits v1 only. v1 has no
        // signed uiActions, so runtime registrations remain diagnostic and
        // produce agreement warnings. R4's v2 activation must supply its
        // VerifiedPackageV2::ui_actions() here, never trust page declarations.
        let catalog = ViewCatalog::from_observation(&[], observation).map_err(error)?;
        Ok(LocalDaemonResponse::AppViewInspected {
            installation_id: binding.installation,
            target_id: target,
            generation: binding.generation.to_string(),
            development,
            catalog_json: serde_json::to_string(&catalog)
                .map_err(|_| error("The App catalog could not be serialized; reload the view"))?,
        })
    }

    async fn check_inspected_release(
        &self,
        room: &str,
        binding: &AppViewBinding,
    ) -> Result<bool, DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let binding = binding.clone();
        let room = room.to_owned();
        tokio::task::spawn_blocking(move || {
            let record = store
                .get_app_installation(&binding.owner, &binding.installation)
                .map_err(|_| error("The App installation is unavailable; check /app status"))?;
            if !record.visible_in_room(Some(&room)) {
                return Err(error("The DEV installation belongs to another Room"));
            }
            let release = store
                .active_app_release(&binding.owner, &binding.installation)
                .map_err(|_| {
                    error("The App release is unavailable or no longer trusted; check /app status")
                })?;
            release
                .verify()
                .map_err(|_| error("The App release failed verification; reinstall it"))?;
            if release.generation() != binding.generation {
                return Err(error(
                    "This App view uses an old generation; reopen it before /app actions",
                ));
            }
            Ok(record.channel
                == chariox_app_runtime::installation::channel::InstallationChannel::Dev)
        })
        .await
        .map_err(|_| error("App storage is unavailable; retry /app actions"))?
    }
}
