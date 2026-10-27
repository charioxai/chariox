//! Owner-scoped native and kernel-browser views over shared request transport.
use super::{
    app_view_runtime::{origin_label, view_assets},
    KernelRuntimeState,
};
use crate::{
    error::DaemonError,
    local::*,
    runtime::{app_views::AppViewBinding, command::KernelCommand},
};

impl KernelRuntimeState {
    // MD-APP/MD-4: choose the lane before allocating its browser/approval futures.
    pub(crate) fn execute_user_app_view_request<'a>(
        &'a self,
        command: &'a KernelCommand,
        request: &'a LocalDaemonRequest,
    ) -> futures_util::future::BoxFuture<'a, Option<Result<LocalDaemonResponse, DaemonError>>> {
        if !matches!(
            request,
            LocalDaemonRequest::OpenUserAppView(_)
                | LocalDaemonRequest::ListUserAppViews(_)
                | LocalDaemonRequest::CloseUserAppView(_)
                | LocalDaemonRequest::GetUserAppViewFrontend(_)
                | LocalDaemonRequest::CallUserAppView(_)
                | LocalDaemonRequest::SubscribeUserAppViews(_)
                | LocalDaemonRequest::AnswerUserDomainInteraction(_)
        ) {
            return Box::pin(async { None });
        }
        Box::pin(self.execute_user_app_view_request_selected(command, request))
    }

    async fn execute_user_app_view_request_selected(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        // This is a human frontend channel, never an agent/host escalation.
        if !command.is_terminal_caller() {
            return Some(Err(DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::NotGranted,
            }));
        }
        let owner = match crate::runtime::app_control::owner(command) {
            Ok(owner) => owner,
            Err(AppRequestErrorCode::Unauthorized | AppRequestErrorCode::NotFound) => {
                return Some(Err(DaemonError::UserDomainRefused {
                    reason: crate::error::UserDomainRefusalReason::NotGranted,
                }))
            }
            Err(code) => return Some(Ok(crate::runtime::app_control::failed(code))),
        };
        Some(
            match Box::pin(self.user_app_view_request(&owner, command, request)).await {
                Ok(LocalDaemonResponse::AppRequestFailed {
                    code: AppRequestErrorCode::NotFound | AppRequestErrorCode::Unauthorized,
                }) => Err(DaemonError::UserDomainRefused {
                    reason: crate::error::UserDomainRefusalReason::NotGranted,
                }),
                result => result,
            },
        )
    }

    async fn user_app_view_request(
        &self,
        owner: &str,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let instances = self.app_control().user_views();
        let failed = crate::runtime::app_control::failed;
        let mut response = match request {
            LocalDaemonRequest::OpenUserAppView(request) => {
                match self
                    .open_user_app_view(owner, &request.installation_id, request.host)
                    .await?
                {
                    Ok((view, frontend)) => {
                        Ok(LocalDaemonResponse::UserAppViewOpened { view, frontend })
                    }
                    Err(code) => return Ok(failed(code)),
                }
            }
            LocalDaemonRequest::ListUserAppViews(_) => {
                Ok(LocalDaemonResponse::UserAppViewsListed {
                    views: instances.list(owner),
                })
            }
            LocalDaemonRequest::CloseUserAppView(request) => {
                if instances.get(owner, &request.view_id).is_none() {
                    return Ok(failed(AppRequestErrorCode::NotFound));
                }
                self.close_user_app_view(owner, &request.view_id).await;
                Ok(LocalDaemonResponse::UserAppViewClosed {
                    view_id: request.view_id.clone(),
                })
            }
            LocalDaemonRequest::GetUserAppViewFrontend(request) => {
                let Some((binding, view)) = instances.get(owner, &request.view_id) else {
                    return Ok(failed(AppRequestErrorCode::NotFound));
                };
                let (current, frontend) =
                    match self.user_app_frontend(owner, &binding.installation).await {
                        Ok(view) => view,
                        Err(code) => return Ok(failed(code)),
                    };
                if current.generation != binding.generation
                    || instances.get(owner, &request.view_id).is_none()
                {
                    return Ok(failed(AppRequestErrorCode::Conflict));
                }
                Ok(LocalDaemonResponse::UserAppViewFrontend { view, frontend })
            }
            LocalDaemonRequest::CallUserAppView(request) => {
                if request.method.is_empty() || request.method.len() > 256 {
                    return Ok(failed(AppRequestErrorCode::InvalidRequest));
                }
                let Some((binding, _)) = instances.get(owner, &request.view_id) else {
                    return Ok(failed(AppRequestErrorCode::NotFound));
                };
                let mut tracked =
                    self.app_control()
                        .views()
                        .track_call(&request.view_id, &request.view_id, None);
                // A Close between lookup and tracking wins, too.
                if instances.get(owner, &request.view_id).is_none() {
                    self.app_control().views().forget_session(&request.view_id);
                    self.app_control().views().keep_pumping(&request.view_id);
                    return Ok(failed(AppRequestErrorCode::NotFound));
                }
                let result = self
                    .invoke_user_app_view_call(
                        owner,
                        &request.view_id,
                        &binding,
                        &request.method,
                        request.input.clone(),
                        &mut tracked,
                        super::app_view_runtime::ViewCaller::Owner(None),
                    )
                    .await;
                match result {
                    Ok(result) => Ok(LocalDaemonResponse::UserAppViewCallResult {
                        result: Some(result),
                        error: None,
                    }),
                    Err(error) => Ok(LocalDaemonResponse::UserAppViewCallResult {
                        result: None,
                        error: Some(error),
                    }),
                }
            }
            LocalDaemonRequest::SubscribeUserAppViews(request) => {
                if request.wait_ms > 25_000 {
                    return Ok(failed(AppRequestErrorCode::InvalidRequest));
                }
                self.owned.sweep_kernel_operation_interactions(false);
                if let Some(after) = request.after {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(request.wait_ms.into()),
                        instances.wait(owner, after),
                    )
                    .await;
                }
                self.owned.sweep_kernel_operation_interactions(false);
                // Capture before the snapshot: any concurrent change is delivered
                // by the next subscription rather than silently skipped.
                let cursor = instances.cursor(owner);
                Ok(LocalDaemonResponse::UserAppViewsChanged {
                    cursor,
                    views: instances.list(owner),
                    interactions: self.owned.user_domain_interactions(owner),
                })
            }
            LocalDaemonRequest::AnswerUserDomainInteraction(request) => {
                if request.interaction_id.len() > 128 || request.choice_id.len() > 128 {
                    return Ok(failed(AppRequestErrorCode::InvalidRequest));
                }
                // Do not enter the passkey gate or answered-prompt cache until
                // the owner has a live detached decision reference. Foreign,
                // fabricated and retired references are indistinguishable.
                if !self
                    .owned
                    .user_domain_interactions(owner)
                    .iter()
                    .any(|interaction| interaction.id() == request.interaction_id)
                {
                    return Err(DaemonError::UserDomainRefused {
                        reason: crate::error::UserDomainRefusalReason::NotGranted,
                    });
                }
                self.answer_terminal_runtime_interaction(
                    "",
                    &request.interaction_id,
                    &request.choice_id,
                    None,
                    Some(owner),
                    request.passkey.as_ref(),
                    request.passkey_remember_minutes,
                    command.caller.connection_class,
                )
                .await?;
                Ok(LocalDaemonResponse::UserDomainInteractionAnswered {
                    interaction_id: request.interaction_id.clone(),
                })
            }
            _ => unreachable!(),
        }?;
        let access: crate::local::UserDomainWindowAccess =
            serde_json::from_value(self.user_domain_window_projection(owner)).map_err(|_| {
                DaemonError::LocalTransport {
                    operation: "user_domain_access",
                    message: "MP-08: window projection unavailable".into(),
                }
            })?;
        match &mut response {
            LocalDaemonResponse::UserAppViewOpened { view, .. }
            | LocalDaemonResponse::UserAppViewFrontend { view, .. } => view.access = Some(access),
            LocalDaemonResponse::UserAppViewsListed { views }
            | LocalDaemonResponse::UserAppViewsChanged { views, .. } => {
                for view in views {
                    view.access = Some(access.clone());
                }
            }
            _ => {}
        }
        Ok(response)
    }

    /// Open an instance of the installation's active release, optionally as a
    /// Tab of the owner's kernel browser. Shared by Open and App Tab restore.
    pub(super) async fn open_user_app_view(
        &self,
        owner: &str,
        installation: &str,
        host: Option<UserAppViewHost>,
    ) -> Result<Result<(UserAppView, AppFrontendBundle), AppRequestErrorCode>, DaemonError> {
        self.open_user_app_view_selected(owner, installation, host, false)
            .await
    }

    pub(super) async fn restore_user_app_view(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Result<(UserAppView, AppFrontendBundle), AppRequestErrorCode>, DaemonError> {
        self.open_user_app_view_selected(
            owner,
            installation,
            Some(UserAppViewHost::KernelBrowser),
            true,
        )
        .await
    }

    async fn open_user_app_view_selected(
        &self,
        owner: &str,
        installation: &str,
        host: Option<UserAppViewHost>,
        restoring: bool,
    ) -> Result<Result<(UserAppView, AppFrontendBundle), AppRequestErrorCode>, DaemonError> {
        let instances = self.app_control().user_views();
        let _opening = if host == Some(UserAppViewHost::KernelBrowser) {
            let opening = instances.opening(owner).await;
            self.seed_user_app_tabs(owner);
            Some(opening)
        } else {
            None
        };
        // An explicit Open that held the guard first already settled this restore.
        if restoring
            && !instances
                .restore_batch(owner)
                .iter()
                .any(|a| a == installation)
        {
            return Ok(Err(AppRequestErrorCode::Conflict));
        }
        let (binding, frontend) = match self.user_app_frontend(owner, installation).await {
            Ok(view) => view,
            Err(code) => return Ok(Err(code)),
        };
        if host == Some(UserAppViewHost::KernelBrowser) && !restoring {
            if let Some(view) =
                Box::pin(self.focus_user_app_browser(owner, &binding, &frontend)).await
            {
                return Ok(Ok((view, frontend)));
            }
        }
        let mut view =
            match instances.open(owner, binding.clone(), &origin_label(owner, installation)) {
                Ok(view) => view,
                Err(code) => return Ok(Err(code)),
            };
        self.app_control()
            .views()
            .register(&view.view_id, &view.view_id, binding.clone());
        if host == Some(UserAppViewHost::KernelBrowser) {
            match self
                .open_user_app_browser(owner, &view, &frontend, restoring)
                .await
            {
                Ok(opened) => view = opened,
                Err(error) => {
                    self.forget_user_app_view(owner, &view.view_id);
                    return Err(error);
                }
            }
        }
        // Uninstall/update can commit while assets are read or the host opens.
        if !self
            .user_app_generation_live(owner, installation, binding.generation)
            .await
            || instances.get(owner, &view.view_id).is_none()
        {
            self.retire_user_app_browser(owner, &view.view_id, restoring)
                .await;
            return Ok(Err(AppRequestErrorCode::Conflict));
        }
        Ok(Ok((view, frontend)))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn invoke_user_app_view_call(
        &self,
        owner: &str,
        view_id: &str,
        binding: &AppViewBinding,
        method: &str,
        input: serde_json::Value,
        tracked: &mut crate::runtime::app_views::ViewCall,
        caller: super::app_view_runtime::ViewCaller<'_>,
    ) -> Result<serde_json::Value, crate::runtime::browser_controller_app_view::BrowserAppViewError>
    {
        if self
            .app_control()
            .user_views()
            .get(owner, view_id)
            .is_none()
            || tracked.is_cancelled()
        {
            return Err(super::app_view_runtime::view_error(
                "CANCELLED",
                "App view closed",
            ));
        }
        if let super::app_view_runtime::ViewCaller::Agent(agent) = &caller {
            let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
            if !self
                .user_domain_owner_aliases(&user)
                .iter()
                .any(|alias| alias == owner)
            {
                return Err(super::app_view_runtime::agent_not_granted());
            }
            let grant = self
                .owned
                .kernel_browser_host
                .retain_resource_grant(
                    &user,
                    agent.id(),
                    UserDomainResource::AppView {
                        view_id: view_id.into(),
                    },
                )
                .map_err(|_| super::app_view_runtime::agent_not_granted())?;
            tracked.retain_browser_grant(grant);
        }
        if method == "chariox.panel" {
            return Ok(serde_json::json!({"placement": "none", "minimized": false}));
        }
        self.invoke_app_view_tool(caller, binding, method, input, tracked)
            .await
    }

    async fn user_app_generation_live(
        &self,
        owner: &str,
        installation: &str,
        generation: u64,
    ) -> bool {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || {
            store
                .active_app_release(&owner, &installation)
                .is_ok_and(|release| release.generation() == generation)
        })
        .await
        .unwrap_or(false)
    }

    pub(super) async fn user_app_frontend(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(AppViewBinding, AppFrontendBundle), AppRequestErrorCode> {
        if installation.is_empty() || installation.len() > 128 {
            return Err(AppRequestErrorCode::InvalidRequest);
        }
        let _permit = self.app_control().try_admit()?;
        let store = self.owned.durable_state_store.clone();
        let (view_owner, view_installation) = (owner.to_owned(), installation.to_owned());
        let assets = tokio::task::spawn_blocking(move || {
            store.app_view_assets(&view_owner, &view_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| {
            use crate::durable_state::app_view_assets::AppViewAssetsError;
            match error {
                AppViewAssetsError::NotFound | AppViewAssetsError::Inactive { .. } => {
                    AppRequestErrorCode::NotFound
                }
                AppViewAssetsError::TooLarge => AppRequestErrorCode::LimitExceeded,
                _ => AppRequestErrorCode::StorageUnavailable,
            }
        })?;
        let binding = AppViewBinding {
            owner: owner.into(),
            installation: installation.into(),
            generation: assets.generation,
            panel: Default::default(),
            logical_tab: None,
        };
        let (entry, assets) = view_assets(assets);
        Ok((binding, AppFrontendBundle { entry, assets,
            content_security_policy: "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; sandbox allow-scripts allow-same-origin allow-forms".into(),
            iframe_sandbox: "allow-scripts allow-same-origin allow-forms".into(),
        }))
    }
}
