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
            Ok((publication_id, pinned, plan)) => LocalDaemonResponse::DeploymentAppsPreview {
                publication_id,
                pinned,
                plan,
            },
            Err(code) => LocalDaemonResponse::AppRequestFailed { code },
        }
    }

    async fn deployment_apps_preview(
        &self,
        command: &KernelCommand,
        request: &PreviewDeploymentAppsRequest,
    ) -> Result<(String, bool, Option<serde_json::Value>), AppRequestErrorCode> {
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
        let Some(mut plan) = publication.apps().cloned() else {
            // Not prepared yet: the plan the owner's current App set gives.
            let Some((mut plan, capabilities)) = self
                .read_publication_app_plan(&publication, &snapshot, &owner)
                .await
                .map_err(|_| AppRequestErrorCode::Conflict)?
            else {
                return Ok((id, false, None));
            };
            for app in plan["apps"].as_array_mut().into_iter().flatten() {
                let installation = app["installation_id"].as_str().unwrap_or_default();
                app["capabilities"] = capabilities.get(installation).cloned().unwrap_or_default();
            }
            return Ok((id, false, Some(plan)));
        };
        // Pinned: the pinned releases' capabilities, from the release store.
        for app in plan["apps"].as_array_mut().into_iter().flatten() {
            let text = |key: &str| app[key].as_str().unwrap_or_default().to_owned();
            let (store, reader) = (self.owned.durable_state_store.clone(), owner.clone());
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
        Ok((id, true, Some(plan)))
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
        let plan = publication
            .apps()
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
            .map_err(|_| AppRequestErrorCode::Busy)?;
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
