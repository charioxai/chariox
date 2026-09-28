//! Protocol 367: `PrepareDeploymentApps` asks the owner once to deploy a
//! workflow together with the Apps of its publication's pinned App plan. One
//! kernel-operation interaction lists each App release with its capabilities
//! and each generator connection the copy will use; the answer is recorded
//! durably. The install policy (`kernel_deployment_consent:<interaction>`)
//! reads that record; this request cannot approve anything by itself.
//! `PreviewDeploymentApps` shows the same plan read-only, before any export.
use super::KernelRuntimeState;
use crate::durable_state::app_installation_operations::{
    ConsentStatus, ConsentedApp, DeploymentConsent, CONSENT_TTL_MS,
};
use crate::local::{
    AppRequestErrorCode, DeploymentAppsConsent, DeploymentAppsConsentStatus, LocalDaemonResponse,
    PrepareDeploymentAppsRequest, PreviewDeploymentAppsRequest,
};
use crate::runtime::{app_operation_budget::AppOperationBudget, command::KernelCommand};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};

/// Publication id, whether a release was prepared, the plan the next release
/// packages and the requested release's plan.
type PreviewedApps = (
    String,
    bool,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
);

impl KernelRuntimeState {
    pub(super) async fn prepare_deployment_apps(
        &self,
        command: &KernelCommand,
        request: &PrepareDeploymentAppsRequest,
    ) -> LocalDaemonResponse {
        match self.deployment_apps_consent(command, request).await {
            Ok(consent) => LocalDaemonResponse::DeploymentAppsConsent {
                consent: DeploymentAppsConsent {
                    request_id: consent.request_id,
                    interaction_id: consent.interaction_id,
                    deployment_id: consent.deployment_id,
                    release_id: consent.release_id,
                    package_digest: consent.package_digest,
                    status: match consent.status {
                        ConsentStatus::Pending => DeploymentAppsConsentStatus::AwaitingApproval,
                        ConsentStatus::Approved => DeploymentAppsConsentStatus::Approved,
                        ConsentStatus::Declined => DeploymentAppsConsentStatus::Declined,
                        ConsentStatus::Expired => DeploymentAppsConsentStatus::Expired,
                    },
                    expires_at_ms: consent.expires_at_ms,
                },
            },
            Err(code) => LocalDaemonResponse::AppRequestFailed { code },
        }
    }

    pub(super) async fn preview_deployment_apps(
        &self,
        command: &KernelCommand,
        request: &PreviewDeploymentAppsRequest,
    ) -> LocalDaemonResponse {
        match self.deployment_apps_preview(command, request).await {
            Ok((publication_id, pinned, plan, release_plan)) => {
                LocalDaemonResponse::DeploymentAppsPreview {
                    publication_id,
                    pinned,
                    plan,
                    release_plan,
                }
            }
            Err(code) => LocalDaemonResponse::AppRequestFailed { code },
        }
    }

    async fn deployment_apps_preview(
        &self,
        command: &KernelCommand,
        request: &PreviewDeploymentAppsRequest,
    ) -> Result<PreviewedApps, AppRequestErrorCode> {
        let owner = crate::runtime::app_control::owner(command)?;
        if !identity(&request.session_id) || !identity(&request.publication_ref) {
            return Err(AppRequestErrorCode::InvalidRequest);
        }
        let (publication, snapshot) = {
            let sessions = self.owned.session_store.read();
            let publication = sessions
                .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)
                .map_err(|_| AppRequestErrorCode::NotFound)?;
            let snapshot = sessions
                .resolve_workflow_publication_snapshot(&request.session_id, publication.id())
                .map_err(|_| AppRequestErrorCode::NotFound)?
                .ok_or(AppRequestErrorCode::NotFound)?;
            (publication, snapshot)
        };
        if publication.created_by_user_id() != owner {
            return Err(AppRequestErrorCode::Unauthorized);
        }
        let id = publication.id().to_owned();
        let prepared = publication.apps().is_some();
        // The release's recorded plan, with its releases' stored capabilities.
        let release_plan = match request.package_digest.as_deref() {
            Some(digest) => match publication.release_app_plan(digest).cloned() {
                Some(mut plan) => {
                    self.stored_plan_capabilities(&owner, &mut plan).await?;
                    Some(plan)
                }
                None => return Err(AppRequestErrorCode::NotFound),
            },
            None => None,
        };
        // Protocol 368: the plan the next release packages, from the owner's
        // current App set.
        let current = match self
            .read_publication_app_plan(&publication, &snapshot, &owner)
            .await
        {
            Ok(current) => current,
            // A past release's plan stays readable when the current App set
            // cannot give one (an App the workflow uses was uninstalled).
            Err(_) if release_plan.is_some() => None,
            Err(_) => return Err(AppRequestErrorCode::Conflict),
        };
        let Some((mut plan, capabilities)) = current else {
            return Ok((id, prepared, None, release_plan));
        };
        for app in plan["apps"].as_array_mut().into_iter().flatten() {
            let installation = app["installation_id"].as_str().unwrap_or_default();
            app["capabilities"] = capabilities.get(installation).cloned().unwrap_or_default();
        }
        Ok((id, prepared, Some(plan), release_plan))
    }

    /// Adds each planned release's capabilities, from the release store.
    async fn stored_plan_capabilities(
        &self,
        owner: &str,
        plan: &mut serde_json::Value,
    ) -> Result<(), AppRequestErrorCode> {
        for app in plan["apps"].as_array_mut().into_iter().flatten() {
            let text = |key: &str| app[key].as_str().unwrap_or_default().to_owned();
            let (store, reader) = (self.owned.durable_state_store.clone(), owner.to_owned());
            let (publisher, key, digest) = (
                text("publisher_id"),
                text("publisher_key_id"),
                text("package_digest"),
            );
            app["capabilities"] = tokio::task::spawn_blocking(move || {
                store.stored_app_release_capabilities(&reader, &publisher, &key, &digest)
            })
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(AppRequestErrorCode::from)?;
        }
        Ok(())
    }

    async fn deployment_apps_consent(
        &self,
        command: &KernelCommand,
        request: &PrepareDeploymentAppsRequest,
    ) -> Result<DeploymentConsent, AppRequestErrorCode> {
        let owner = crate::runtime::app_control::owner(command)?;
        let valid = [
            &request.session_id,
            &request.request_id,
            &request.publication_ref,
            &request.deployment_id,
            &request.release_id,
        ]
        .iter()
        .all(|value| identity(value))
            && request.package_digest.len() == 71
            && request
                .package_digest
                .strip_prefix("sha256:")
                .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()));
        if !valid {
            return Err(AppRequestErrorCode::InvalidRequest);
        }
        if !self.app_install_session_member(&request.session_id, &owner) {
            return Err(AppRequestErrorCode::Unauthorized);
        }
        let publication = self
            .owned
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)
            .map_err(|_| AppRequestErrorCode::NotFound)?;
        if publication.created_by_user_id() != owner {
            return Err(AppRequestErrorCode::Unauthorized);
        }
        let consent = DeploymentConsent {
            request_id: request.request_id.clone(),
            interaction_id: format!("app_deploy_{:032x}", rand::random::<u128>()),
            session_id: request.session_id.clone(),
            publication_id: publication.id().to_owned(),
            deployment_id: request.deployment_id.clone(),
            release_id: request.release_id.clone(),
            package_digest: request.package_digest.clone(),
            apps: Vec::new(),
            status: ConsentStatus::Pending,
            expires_at_ms: crate::session::unix_epoch_ms() + CONSENT_TTL_MS,
        };
        // The same request replays its consent and reports the answer.
        let store = self.owned.durable_state_store.clone();
        let (replay_owner, request_id) = (owner.clone(), request.request_id.clone());
        let existing = tokio::task::spawn_blocking(move || {
            store.deployment_consent(&replay_owner, &request_id)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
        if let Some(existing) = existing {
            return if existing.same_request(&consent) {
                Ok(existing)
            } else {
                Err(AppRequestErrorCode::Conflict)
            };
        }
        // Protocol 368: exactly the plan this release was exported with.
        let plan = publication
            .release_app_plan(&request.package_digest)
            .cloned()
            .ok_or(AppRequestErrorCode::InvalidRequest)?;
        let mut apps = Vec::new();
        let mut lines = vec![format!(
            "Deploy `{}` (release `{}`) with its Apps? Its copy gets its own installation of each App below, from exactly this release, and no App data:",
            request.deployment_id, request.release_id
        )];
        for app in plan["apps"].as_array().into_iter().flatten() {
            let text = |key: &str| app[key].as_str().unwrap_or_default().to_owned();
            // The pinned release, re-verified against the owner's current
            // publisher trust; its capabilities are what the owner reviews.
            let permit = self.app_control().try_admit()?;
            let prepared = self
                .app_control()
                .preparation()
                .prepare_release(owner.clone(), text("package_digest"), permit)
                .await
                .map_err(|_| AppRequestErrorCode::Conflict)?;
            let candidate = prepared
                .candidate(&owner)
                .map_err(|_| AppRequestErrorCode::Conflict)?;
            let (release, review) = (candidate.release_metadata(), candidate.review_metadata());
            let consented = ConsentedApp {
                app_id: release.app_id.clone(),
                publisher_id: release.publisher_id.clone(),
                publisher_key_fingerprint: review["keyFingerprint"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                package_digest: release.package_digest.clone(),
                capabilities_digest: release.capabilities_digest.clone(),
            };
            if consented.app_id != text("app_id")
                || consented.publisher_key_fingerprint != text("publisher_key_fingerprint")
                || consented.capabilities_digest != text("capabilities_digest")
            {
                return Err(AppRequestErrorCode::Conflict);
            }
            lines.push(format!(
                "- {} {} from {} (key {}), capabilities: {}",
                release.app_id,
                release.version,
                release.publisher_id,
                consented.publisher_key_fingerprint,
                review["capabilities"]
            ));
            for route in app["inbox_routes"].as_array().into_iter().flatten() {
                let connection = &route["connection"];
                if connection.is_object() {
                    lines.push(format!(
                        "  shares connection {} of {} ({}) for its inbox route {}",
                        connection["connection_id"].as_str().unwrap_or_default(),
                        connection["generator_id"].as_str().unwrap_or_default(),
                        connection["connection_scope"].as_str().unwrap_or_default(),
                        route["route_id"].as_str().unwrap_or_default(),
                    ));
                }
            }
            for connection in app["connections"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "  shares connection {} of {} to act with: {}",
                    connection["connection_id"].as_str().unwrap_or_default(),
                    connection["generator_id"].as_str().unwrap_or_default(),
                    connection["actions"],
                ));
            }
            apps.push(consented);
        }
        let consent = DeploymentConsent { apps, ..consent };
        // Protocol 368: a release with exactly the App releases the owner
        // already approved for this deployment is approved without asking
        // again; its install window starts now.
        let store = self.owned.durable_state_store.clone();
        let (reader, deployment, apps) = (
            owner.clone(),
            consent.deployment_id.clone(),
            consent.apps.clone(),
        );
        let approved_before = tokio::task::spawn_blocking(move || {
            store.deployment_apps_approved_before(&reader, &deployment, &apps)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
        if approved_before {
            let store = self.owned.durable_state_store.clone();
            let (writer, pending) = (owner.clone(), consent.clone());
            return tokio::task::spawn_blocking(move || {
                let budget = || AppOperationBudget::from_supervisor(|| false);
                let recorded = store.begin_deployment_consent(&writer, pending, budget())?;
                if recorded.status == ConsentStatus::Pending {
                    store.decide_deployment_consent(
                        &writer,
                        &recorded.interaction_id,
                        true,
                        budget(),
                    )?;
                }
                store
                    .deployment_consent(&writer, &recorded.request_id)?
                    .ok_or(crate::durable_state::app_installation_operations::InstallOperationError::Storage)
            })
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(|_| AppRequestErrorCode::StorageUnavailable);
        }
        let interaction = RuntimeInteraction::for_kernel_operation(
            consent.interaction_id.clone(),
            format!("deploy:{}", consent.deployment_id),
            "Deploy with Apps",
            lines.join("\n"),
            vec![
                RuntimeInteractionChoice::new("approve", "Deploy", "approve", None),
                RuntimeInteractionChoice::new("decline", "Cancel", "decline", None),
            ],
        );
        // One decision per deployment at a time: another release's consent
        // still awaiting the owner is answered first.
        let answer = self
            .create_kernel_operation_interaction(&consent.session_id, &owner, interaction)
            .await
            .map_err(|error| {
                if super::runtime_interaction_owned_state::interaction_waits(&error) {
                    return AppRequestErrorCode::Busy;
                }
                crate::logging::warn_with_fields(
                    "daemon.app_deployment_consent",
                    "the deployment consent prompt could not be raised",
                    serde_json::json!({"error": error.to_string()}),
                );
                AppRequestErrorCode::StorageUnavailable
            })?;
        let store = self.owned.durable_state_store.clone();
        let (begin_owner, pending) = (owner.clone(), consent.clone());
        let recorded = tokio::task::spawn_blocking(move || {
            store.begin_deployment_consent(
                &begin_owner,
                pending,
                AppOperationBudget::from_supervisor(|| false),
            )
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
        let recorded = match recorded {
            Ok(recorded) if recorded.interaction_id == consent.interaction_id => recorded,
            other => {
                // A concurrent replay recorded its own prompt first.
                let _ = self
                    .timeout_runtime_interaction(&consent.session_id, &consent.interaction_id)
                    .await;
                return other.map_err(|_| AppRequestErrorCode::Conflict);
            }
        };
        // The answer is recorded by the kernel, bounded by the prompt's expiry.
        let (owned, store) = (self.owned.clone(), self.owned.durable_state_store.clone());
        let (session, interaction_id) = (consent.session_id, consent.interaction_id);
        tokio::spawn(async move {
            let wait = std::time::Duration::from_millis(CONSENT_TTL_MS);
            match tokio::time::timeout(wait, answer).await {
                Ok(Ok(value)) => {
                    let approved = value.status == "answered"
                        && value.choice_id.as_deref() == Some("approve")
                        && value.reply.as_deref() == Some("approve");
                    let _ = tokio::task::spawn_blocking(move || {
                        store.decide_deployment_consent(
                            &owner,
                            &interaction_id,
                            approved,
                            AppOperationBudget::from_supervisor(|| false),
                        )
                    })
                    .await;
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    let _ = owned.timeout_runtime_interaction(&session, &interaction_id);
                }
            }
        });
        Ok(recorded)
    }
}

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
