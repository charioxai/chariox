//! Observe an owned App Tab using its retained binding and signed release.
use super::KernelRuntimeState;
use crate::{
    error::DaemonError,
    local::{InspectAppViewRequest, LocalDaemonResponse},
    runtime::{app_views::AppViewBinding, browser_controller_app_view::BrowserAppViewRequest},
};
use chariox_app_runtime::view_catalog::{ViewCatalog, ViewObservation};

use super::app_view_runtime::open_error as error;

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn kernel_browser_host_for_test(
        &self,
    ) -> &crate::runtime::kernel_browser_host::KernelBrowserHost {
        &self.owned.kernel_browser_host
    }

    pub(super) async fn inspect_app_view(
        &self,
        owner: String,
        request: &InspectAppViewRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(tab) = request
            .target_id
            .as_deref()
            .filter(|tab| tab.starts_with("host-tab-"))
        {
            return self.inspect_user_browser_app(&owner, tab).await;
        }
        let tab = match &request.target_id {
            Some(target) if !target.is_empty() && target.len() <= 256 => target.clone(),
            Some(_) => {
                return Err(error(
                    "Invalid App target; open the App view and retry /app actions",
                ));
            }
            None => self
                .room_environment_snapshot(&request.session_id)
                .map_err(|_| {
                    error("The Room environment is unavailable; start it before /app actions")
                })?
                .focused_tab_id
                .ok_or_else(|| error("Focus an open App view before using /app actions"))?,
        };
        let target = self
            .room_environment_controller_tab_binding(&request.session_id, &tab)
            .map_err(|_| error("This Tab is not an open App view; use /app open first"))?
            .runtime_target_id;
        let views = self.app_control().views().clone();
        let (binding, reconnecting) = views
            .binding_state(&request.session_id, &target)
            .ok_or_else(|| error("This Tab is not an open App view; use /app open first"))?;
        if reconnecting || binding.owner != owner {
            return Err(error(
                "The App view is unavailable or reloading; retry after opening it",
            ));
        }
        let (development, declared) = self
            .check_inspected_release(Some(&request.session_id), &binding)
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
        self.check_inspected_release(Some(&request.session_id), &binding)
            .await?;
        let catalog = ViewCatalog::from_observation(&declared, observation).map_err(error)?;
        Ok(LocalDaemonResponse::AppViewInspected {
            installation_id: binding.installation,
            target_id: tab,
            generation: binding.generation.to_string(),
            development,
            catalog_json: serde_json::to_string(&catalog)
                .map_err(|_| error("The App catalog could not be serialized; reload the view"))?,
        })
    }

    async fn inspect_user_browser_app(
        &self,
        owner: &str,
        tab: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        use super::app_view_host::{AppViewHost, KernelBrowserAppViewHost};
        if tab.len() > 256 {
            return Err(error("Invalid App target; reopen the view"));
        }
        let views = self.app_control().user_views();
        let view = views
            .list(owner)
            .into_iter()
            .find(|view| {
                view.browser
                    .as_ref()
                    .is_some_and(|browser| browser.tab_id == tab)
            })
            .ok_or_else(|| error("This Tab is not an open App view; use /app open first"))?;
        let retained = views
            .get(owner, &view.view_id)
            .ok_or_else(|| error("The App view closed; reopen it"))?;
        let (binding, view) = &retained;
        let browser = view
            .browser
            .as_ref()
            .ok_or_else(|| error("This App has no kernel-browser view"))?;
        let target = views
            .browser_target(owner, &view.view_id)
            .ok_or_else(|| error("The App view is reloading; retry after opening it"))?;
        let (development, declared) = self.check_inspected_release(None, binding).await?;
        let value = KernelBrowserAppViewHost {
            state: self,
            owner,
            generation: Some(browser.generation),
        }
        .command(BrowserAppViewRequest::Inspect {
            target_id: target.clone(),
        })
        .await?
        .ok_or_else(|| error("The App browser is unavailable; reopen it"))?;
        let observation: ViewObservation = serde_json::from_value(value)
            .map_err(|_| error("The App browser returned an invalid observation"))?;
        if views.get(owner, &view.view_id).as_ref() != Some(&retained)
            || views.browser_target(owner, &view.view_id).as_ref() != Some(&target)
        {
            return Err(error(
                "The App view changed during inspection; retry /app actions",
            ));
        }
        self.check_inspected_release(None, binding).await?;
        let catalog = ViewCatalog::from_observation(&declared, observation).map_err(error)?;
        Ok(LocalDaemonResponse::AppViewInspected {
            installation_id: binding.installation.clone(),
            target_id: tab.into(),
            generation: binding.generation.to_string(),
            development,
            catalog_json: serde_json::to_string(&catalog)
                .map_err(|_| error("The App catalog could not be serialized; reload the view"))?,
        })
    }

    pub(super) async fn check_inspected_release(
        &self,
        room: Option<&str>,
        binding: &AppViewBinding,
    ) -> Result<(bool, Vec<chariox_app_package::v2::UiActionDeclaration>), DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let binding = binding.clone();
        let room = room.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let record = store
                .get_app_installation(&binding.owner, &binding.installation)
                .map_err(|_| error("The App installation is unavailable; check /app status"))?;
            if room
                .as_deref()
                .is_some_and(|room| !record.visible_in_room(Some(room)))
            {
                return Err(error("The DEV installation belongs to another Room"));
            }
            let release = store
                .active_app_release(&binding.owner, &binding.installation)
                .map_err(|_| {
                    error("The App release is unavailable or no longer trusted; check /app status")
                })?;
            let verified = release
                .verify()
                .map_err(|_| error("The App release failed verification; reinstall it"))?;
            if release.generation() != binding.generation {
                return Err(error(
                    "This App view uses an old generation; reopen it before /app actions",
                ));
            }
            let catalog = release
                .event_catalog(&verified)
                .map_err(|_| error("The signed App catalog is unavailable; reinstall it"))?;
            Ok((
                record.channel
                    == chariox_app_runtime::installation::channel::InstallationChannel::Dev,
                catalog.app_catalog().ui_actions().to_vec(),
            ))
        })
        .await
        .map_err(|_| error("App storage is unavailable; retry /app actions"))?
    }

    /// The App and publisher a person confirms for, read from the verified active release
    /// of this exact generation; never from the page.
    pub(super) async fn app_confirmation_frame(
        &self,
        binding: &AppViewBinding,
        action_name: &str,
    ) -> Result<crate::local::AppConfirmationFrame, DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let binding = binding.clone();
        let action_name = action_name.to_owned();
        tokio::task::spawn_blocking(move || {
            let release = store
                .active_app_release(&binding.owner, &binding.installation)
                .map_err(|_| {
                    error("The App release is unavailable or no longer trusted; check /app status")
                })?;
            let verified = release
                .verify()
                .map_err(|_| error("The App release failed verification; reinstall it"))?;
            if release.generation() != binding.generation {
                return Err(error(
                    "This App view uses an old generation; reopen it before App actions",
                ));
            }
            use chariox_app_package::VerifiedApp;
            let app_id = verified.manifest().app_id.to_owned();
            let (display_name, publisher_name) = match verified.release_manifest() {
                chariox_app_package::ReleaseManifest::Wasm(manifest) => {
                    (manifest.display_name, manifest.publisher.name)
                }
                chariox_app_package::ReleaseManifest::Node(manifest) => {
                    (None, manifest.publisher.name)
                }
            };
            Ok(crate::local::AppConfirmationFrame {
                installation_id: binding.installation,
                app_name: display_name.unwrap_or_else(|| app_id.clone()),
                app_id,
                publisher_id: release.publisher_id(),
                publisher_name,
                action_name,
            })
        })
        .await
        .map_err(|_| error("App storage is unavailable; retry the action"))?
    }
}
