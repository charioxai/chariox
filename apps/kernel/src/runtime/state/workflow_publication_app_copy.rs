//! P1.20: an App-bound workflow deployed on its owner's kernel (a bound
//! `local_runtime` deployment) runs as a pinned independent copy. The copy is
//! a hidden session materialized from the publication snapshot (runtime key
//! `deployment:<deployment>:<release>`, same publication kind) whose agents use
//! the deployment's own installations of the pinned Apps — installed from the
//! local release store under the owner's deployment consent and tagged with the
//! deployment — configured as the plan says: automations on the copy's
//! publication, the same connection grants, and the same inbox routes. A copy's
//! route takes over from the owner's: the owner's route on the same event
//! interest pauses while the copy's is active and resumes once it is gone.
//! Everything here is idempotent: bind, recovery and restart apply it again.
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use tokio::time::{sleep, Duration, Instant};

use super::workflow_publication_runtime_lifecycle::DEPLOYMENT_COPY_KEY_PREFIX as COPY_KEY_PREFIX;
use super::KernelRuntimeState;
use crate::durable_state::app_inbox::{AppInboxOperation, AppInboxOutcome};
use crate::durable_state::app_installation_operations::{
    DeploymentInstall, InstallOperation, InstallOperationError, InstallPhase, UpdateTarget,
};
use crate::error::DaemonError;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::local::{
    AppAutomationStatus, AppInboxConnection, AppInboxRouteRequest, AppRequestErrorCode,
    AppSetInstallation, ConfigureAppAutomationRequest, CreateAppInboxRouteRequest,
    DisableAppAutomationRequest, GrantAppConnectionRequest, LocalDaemonRequest,
    MaterializeWorkflowPublicationRequest, RevokeAppConnectionRequest, UninstallAppRequest,
};
use crate::session::WorkflowPublicationDefinition;

#[cfg(not(test))]
const COPY_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(test)]
const COPY_INSTALL_TIMEOUT: Duration = Duration::from_secs(20);
const COPY_INSTALL_POLL: Duration = Duration::from_millis(100);
const COPY_INSTALL_ATTEMPTS: u32 = 8;

/// The session a deployment copy runs in.
pub(super) struct DeploymentAppCopy {
    pub(super) session_id: String,
}

fn copy_key(deployment_id: &str, release_id: &str) -> String {
    format!("{COPY_KEY_PREFIX}{deployment_id}:{release_id}")
}

/// Install requests of one deployment, release and App are named
/// `<prefix><attempt>`: a retry replays the latest; a spent one (failed,
/// cancelled, or whose copy was removed since, as after a Stop) moves on to the
/// next attempt.
fn copy_request_prefix(deployment_id: &str, release_id: &str, app_id: &str) -> String {
    let digest = Sha256::digest(
        [
            deployment_id.as_bytes(),
            b"\0",
            release_id.as_bytes(),
            b"\0",
            app_id.as_bytes(),
        ]
        .concat(),
    );
    format!("deploy_{}_", &format!("{digest:x}")[..40])
}

fn copy_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "deploy workflow with its Apps",
        message: message.into(),
    }
}

fn app_error(what: &str, code: AppRequestErrorCode) -> DaemonError {
    copy_error(format!("{what} ({code:?})"))
}

fn text<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// The event interest a generator-fed route claims; the event service keeps
/// one active route per interest.
fn interest(connection: &AppInboxConnection, event_type: &str, version: u32) -> Option<String> {
    chariox_event_protocol::event_interest_key(
        &connection.generator_id,
        event_type,
        version,
        &connection.connection_scope,
        &connection.filter,
    )
    .ok()
}

impl KernelRuntimeState {
    /// Creates or brings up to date the copy of an App-bound publication for a
    /// bound deployment; `None` when the publication uses no App.
    pub(super) async fn ensure_deployment_app_copy(
        &self,
        publication: &WorkflowPublicationDefinition,
        deployment_id: &str,
        release_id: &str,
        package_digest: &str,
    ) -> Result<Option<DeploymentAppCopy>, DaemonError> {
        if publication.apps().is_none() {
            return Ok(None);
        }
        let owner = publication.created_by_user_id().to_owned();
        // A release exported while the publication used no App runs from the
        // source; a copy of another release goes.
        if publication.release_without_apps(package_digest) {
            self.remove_deployment_app_copy(&owner, deployment_id)
                .await?;
            return Ok(None);
        }
        // Protocol 377: the release's own plan, so a rollback restores its Apps.
        let plan = publication
            .release_app_plan(package_digest)
            .ok_or_else(|| {
                copy_error(
                    "this release's App plan is not recorded on this kernel; export it again",
                )
            })?;
        let apps = plan["apps"].as_array().cloned().unwrap_or_default();
        // A release that uses no App runs from the source, as one without a
        // plan: the deployment's copy goes (its installations, routes and
        // sessions) and the owner's routes resume.
        if apps.is_empty() {
            self.remove_deployment_app_copy(&owner, deployment_id)
                .await?;
            return Ok(None);
        }
        let consent = self
            .approved_deployment_consent(&owner, deployment_id, release_id, package_digest)
            .await?;
        let key = copy_key(deployment_id, release_id);
        self.remove_deployment_copy_sessions(&owner, deployment_id, Some(&key))
            .await?;
        let (session_id, agents) = self.materialize_deployment_copy(publication, &key)?;
        let applied = self
            .apply_deployment_app_copy(
                &owner,
                publication,
                (deployment_id, release_id, &consent),
                &apps,
                &session_id,
                &agents,
            )
            .await;
        // A copy that could not be applied takes no occurrences: its runtime
        // does not start, so the owner's routes it would take over resume.
        if applied.is_err() {
            if let Err(error) = self.withdraw_copy_routes(&owner, deployment_id).await {
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to withdraw the routes of a copy that could not be applied",
                    serde_json::json!({
                        "deployment_id": deployment_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
        let balanced = self.balance_owner_inbox_routes(&owner, None).await;
        applied?;
        balanced?;
        Ok(Some(DeploymentAppCopy { session_id }))
    }

    async fn apply_deployment_app_copy(
        &self,
        owner: &str,
        publication: &WorkflowPublicationDefinition,
        (deployment_id, release_id, consent): (&str, &str, &str),
        apps: &[serde_json::Value],
        session_id: &str,
        agents: &BTreeMap<String, String>,
    ) -> Result<(), DaemonError> {
        let owner = owner.to_owned();
        let set = self.app_set_for(&owner).await?;
        let mut copies = BTreeMap::new();
        for app in apps {
            let copy = self
                .ensure_copy_installation(
                    &owner,
                    publication.session_id(),
                    deployment_id,
                    release_id,
                    consent,
                    app,
                    &set,
                )
                .await?;
            copies.insert(text(app, "installation_id").to_owned(), copy);
        }
        // A release that no longer uses an App drops its copy.
        for stale in set.iter().filter(|installation| {
            installation.deployment_id.as_deref() == Some(deployment_id)
                && !copies
                    .values()
                    .any(|id| *id == installation.installation_id)
        }) {
            self.uninstall_deployment_copy(&owner, &stale.installation_id)
                .await?;
        }
        self.remap_copy_app_grants(&owner, agents, &copies).await?;
        let set = self.app_set_for(&owner).await?;
        for app in apps {
            let source = text(app, "installation_id");
            let copy = set
                .iter()
                .find(|installation| Some(&installation.installation_id) == copies.get(source))
                .ok_or_else(|| copy_error(format!("the copy of App `{source}` is not active")))?;
            self.apply_copy_connections(&owner, copy, app).await?;
            self.apply_copy_automations(&owner, copy, app, session_id, publication.id())
                .await?;
            self.apply_copy_routes(&owner, copy, app, &set).await?;
        }
        Ok(())
    }

    /// Removes the copies whose source publication was deleted (or its
    /// session), disabled, stopped or bound to another deployment. `sources`
    /// maps each live source (owner, publication) to the deployment it is
    /// bound to, or `None` while unbound (a bind may be starting).
    pub(super) async fn remove_orphaned_deployment_copies(
        &self,
        sources: &BTreeMap<(String, String), Option<String>>,
    ) {
        let orphans = self
            .owned
            .session_store
            .read()
            .list_all_sessions()
            .into_iter()
            .filter(|session| session.is_hidden())
            .flat_map(|session| {
                let owner = session.owner_user_id().to_owned();
                session
                    .workflow_publications()
                    .iter()
                    .filter_map(|publication| {
                        let key = &publication.runtime_materialization()?.key;
                        let (deployment, _release) =
                            key.strip_prefix(COPY_KEY_PREFIX)?.rsplit_once(':')?;
                        let kept = match sources.get(&(owner.clone(), publication.id().to_owned()))
                        {
                            Some(None) => true,
                            Some(Some(bound)) => bound == deployment,
                            None => false,
                        };
                        (!kept).then(|| (owner.clone(), deployment.to_owned()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        for (owner, deployment_id) in orphans {
            if let Err(error) = self
                .remove_deployment_app_copy(&owner, &deployment_id)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to remove an orphaned deployment copy",
                    serde_json::json!({
                        "deployment_id": deployment_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    /// Removes the inbox routes of a deployment's copies; applying the copy
    /// again creates them anew.
    async fn withdraw_copy_routes(
        &self,
        owner: &str,
        deployment_id: &str,
    ) -> Result<(), DaemonError> {
        for copy in self
            .app_set_for(owner)
            .await?
            .into_iter()
            .filter(|installation| installation.deployment_id.as_deref() == Some(deployment_id))
        {
            for route in &copy.inbox_routes {
                self.copy_request(
                    owner,
                    &copy.installation_id,
                    LocalDaemonRequest::RemoveAppInboxRoute(AppInboxRouteRequest {
                        installation_id: copy.installation_id.clone(),
                        route_id: route.route_id.clone(),
                    }),
                    "an inbox route could not be removed from the copy",
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Removes a deployment's copy: its installations with their data (and so
    /// their routes, automations and connection grants), then its sessions.
    /// The owner's routes the copy had taken over resume.
    pub(super) async fn remove_deployment_app_copy(
        &self,
        owner: &str,
        deployment_id: &str,
    ) -> Result<(), DaemonError> {
        // Hand the routes back first: an occurrence then reaches the copy or
        // the owner, never a paused route.
        self.balance_owner_inbox_routes(owner, Some(deployment_id))
            .await?;
        for copy in self
            .app_set_for(owner)
            .await?
            .into_iter()
            .filter(|installation| installation.deployment_id.as_deref() == Some(deployment_id))
        {
            if let Err(error) = self
                .uninstall_deployment_copy(owner, &copy.installation_id)
                .await
            {
                // The owner's routes are back: a copy left for a later retry
                // must not receive the same occurrences.
                if let Err(withdrawal) = self.withdraw_copy_routes(owner, deployment_id).await {
                    crate::logging::warn_with_fields(
                        "daemon.publication_runtime",
                        "failed to withdraw the routes of a copy that could not be removed",
                        serde_json::json!({
                            "deployment_id": deployment_id,
                            "error": withdrawal.to_string(),
                        }),
                    );
                }
                return Err(error);
            }
        }
        self.remove_deployment_copy_sessions(owner, deployment_id, None)
            .await
    }

    async fn approved_deployment_consent(
        &self,
        owner: &str,
        deployment_id: &str,
        release_id: &str,
        package_digest: &str,
    ) -> Result<String, DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let (owner, deployment, release, digest) = (
            owner.to_owned(),
            deployment_id.to_owned(),
            release_id.to_owned(),
            package_digest.to_owned(),
        );
        tokio::task::spawn_blocking(move || {
            store.approved_deployment_consent(&owner, &deployment, &release, &digest)
        })
        .await
        .map_err(|_| copy_error("the deployment consent could not be read"))?
        .map_err(|_| copy_error("the deployment consent could not be read"))?
        .ok_or_else(|| {
            copy_error(
                "this workflow uses Apps: its owner must approve deploying this release with them first (PrepareDeploymentApps)",
            )
        })
    }

    fn materialize_deployment_copy(
        &self,
        publication: &WorkflowPublicationDefinition,
        key: &str,
    ) -> Result<(String, BTreeMap<String, String>), DaemonError> {
        let (snapshot, source) = {
            let sessions = self.owned.session_store.read();
            (
                sessions
                    .resolve_workflow_publication_snapshot(
                        publication.session_id(),
                        publication.id(),
                    )?
                    .ok_or_else(|| {
                        copy_error("the workflow trigger has no immutable source snapshot")
                    })?,
                sessions.get_session(publication.session_id())?,
            )
        };
        match self.owned.workflow_materialize_publication_as(
            MaterializeWorkflowPublicationRequest {
                publication_id: publication.id().to_owned(),
                snapshot,
                runtime_key: Some(key.to_owned()),
            },
            publication.created_by_user_id(),
            publication.kind(),
            publication.queue_ref().map(str::to_owned),
            // The snapshot stays portable; on its owner's kernel the copy
            // works in the source session's own workspace, as the source's
            // local deployment did.
            Some((
                source.workspace_id().to_owned(),
                source.worktree_id().to_owned(),
            )),
        )? {
            crate::local::LocalDaemonResponse::WorkflowPublicationMaterialized {
                session,
                agent_id_map,
                ..
            } => Ok((session.id().to_owned(), agent_id_map)),
            _ => Err(copy_error("the copy session could not be created")),
        }
    }

    /// The owner's hidden copy sessions of a deployment, except `keep`.
    async fn remove_deployment_copy_sessions(
        &self,
        owner: &str,
        deployment_id: &str,
        keep: Option<&str>,
    ) -> Result<(), DaemonError> {
        let prefix = format!("{COPY_KEY_PREFIX}{deployment_id}:");
        let sessions = self
            .owned
            .session_store
            .read()
            .list_all_sessions()
            .into_iter()
            .filter(|session| {
                session.is_hidden()
                    && session.owner_user_id() == owner
                    && session.workflow_publications().iter().any(|publication| {
                        publication
                            .runtime_materialization()
                            .is_some_and(|materialization| {
                                materialization.key.starts_with(&prefix)
                                    && Some(materialization.key.as_str()) != keep
                            })
                    })
            })
            .map(|session| session.id().to_owned())
            .collect::<Vec<_>>();
        for session_id in sessions {
            self.delete_session_id(&session_id).await?;
        }
        Ok(())
    }

    async fn app_set_for(&self, owner: &str) -> Result<Vec<AppSetInstallation>, DaemonError> {
        self.read_app_set(owner)
            .await
            .map_err(|code| app_error("the App set could not be read", code))
    }

    /// The deployment's installation of a planned App: the existing copy of
    /// the same release, the copy updated to it (migrating a newer data
    /// schema), or a new copy. A release with an older schema fails closed.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_copy_installation(
        &self,
        owner: &str,
        install_session_id: &str,
        deployment_id: &str,
        release_id: &str,
        consent: &str,
        app: &serde_json::Value,
        set: &[AppSetInstallation],
    ) -> Result<String, DaemonError> {
        let (app_id, package_digest) = (text(app, "app_id"), text(app, "package_digest"));
        let update = match set.iter().find(|installation| {
            installation.deployment_id.as_deref() == Some(deployment_id)
                && installation.app_id == app_id
        }) {
            Some(copy) if copy.release.package_digest == package_digest => {
                return Ok(copy.installation_id.clone());
            }
            Some(copy) => {
                // A newer schema migrates the copy's data forward as any App
                // update does; App data is never migrated to an older one.
                let schema = app["schema_version"].as_u64();
                if schema.is_none_or(|schema| schema < u64::from(copy.release.schema_version)) {
                    return Err(copy_error(format!(
                        "App `{app_id}` of this release keeps its data in schema version {}, older than the deployment copy's version {}; stop the deployment to start its Apps anew",
                        schema.unwrap_or_default(),
                        copy.release.schema_version
                    )));
                }
                let store = self.owned.durable_state_store.clone();
                let (read_owner, id) = (owner.to_owned(), copy.installation_id.clone());
                let generation = tokio::task::spawn_blocking(move || {
                    store.get_app_installation(&read_owner, &id)
                })
                .await
                .map_err(|_| copy_error("an App copy could not be read"))?
                .map_err(|_| copy_error("an App copy could not be read"))?
                .generation;
                Some(UpdateTarget {
                    installation_id: copy.installation_id.clone(),
                    expected_generation: generation,
                })
            }
            None => None,
        };
        let deployment = DeploymentInstall {
            consent: consent.to_owned(),
            deployment_id: deployment_id.to_owned(),
        };
        let prefix = copy_request_prefix(deployment_id, release_id, app_id);
        let store = self.owned.durable_state_store.clone();
        let (read_owner, read_prefix) = (owner.to_owned(), prefix.clone());
        let latest = tokio::task::spawn_blocking(move || {
            store.latest_app_install_attempt(&read_owner, &read_prefix)
        })
        .await
        .map_err(|_| copy_error("an App install could not be read"))?
        .map_err(|error| copy_error(format!("an App install could not be read ({error:?})")))?
        .unwrap_or_default();
        let mut attempts = latest..latest.saturating_add(COPY_INSTALL_ATTEMPTS);
        let request_id = loop {
            let Some(attempt) = attempts.next() else {
                return Err(copy_error(format!(
                    "App `{app_id}` could not be installed for the deployment after {COPY_INSTALL_ATTEMPTS} attempts"
                )));
            };
            let request_id = format!("{prefix}{attempt}");
            match self
                .app_control()
                .installs()
                .begin_deployment_install(
                    owner,
                    &request_id,
                    install_session_id,
                    deployment.clone(),
                    update.clone(),
                    package_digest,
                )
                .await
            {
                // The same request for another starting point.
                Err(InstallOperationError::Conflict) => continue,
                Err(error) => {
                    return Err(copy_error(format!(
                        "App `{app_id}` could not be installed for the deployment ({error:?})"
                    )))
                }
                Ok(operation) if self.spent_copy_install(owner, &operation).await? => continue,
                Ok(_) => break request_id,
            }
        };
        let deadline = Instant::now() + COPY_INSTALL_TIMEOUT;
        loop {
            let store = self.owned.durable_state_store.clone();
            let (read_owner, request) = (owner.to_owned(), request_id.clone());
            let operation = tokio::task::spawn_blocking(move || {
                store.first_app_install_status(&read_owner, &request)
            })
            .await
            .map_err(|_| copy_error("an App install could not be read"))?
            .map_err(|error| copy_error(format!("an App install could not be read ({error:?})")))?;
            match operation.phase {
                InstallPhase::Committed => return Ok(operation.token.installation_id),
                InstallPhase::Failed | InstallPhase::Cancelled => {
                    return Err(copy_error(format!(
                        "App `{app_id}` could not be installed for the deployment ({})",
                        operation.failure.as_deref().unwrap_or("cancelled")
                    )));
                }
                // The consent did not cover this release: the owner was asked.
                InstallPhase::AwaitingApproval
                    if !operation.approved && operation.interaction_id.is_some() =>
                {
                    return Err(copy_error(format!(
                        "App `{app_id}` needs its owner's approval before this deployment can run; answer the prompt and deploy again"
                    )));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err(copy_error(format!(
                    "App `{app_id}` did not finish installing for the deployment ({:?}); deploy again to continue",
                    operation.phase
                )));
            }
            sleep(COPY_INSTALL_POLL).await;
        }
    }

    /// A replayed install that cannot give the copy: it failed, was
    /// cancelled, or its installation was removed since.
    async fn spent_copy_install(
        &self,
        owner: &str,
        operation: &InstallOperation,
    ) -> Result<bool, DaemonError> {
        Ok(match operation.phase {
            InstallPhase::Failed | InstallPhase::Cancelled => true,
            InstallPhase::Committed => {
                let store = self.owned.durable_state_store.clone();
                let (read_owner, id) = (owner.to_owned(), operation.token.installation_id.clone());
                tokio::task::spawn_blocking(move || store.get_app_installation(&read_owner, &id))
                    .await
                    .map_err(|_| copy_error("an App copy could not be read"))?
                    .map_or(true, |installation| installation.active.is_none())
            }
            _ => false,
        })
    }

    async fn uninstall_deployment_copy(
        &self,
        owner: &str,
        installation_id: &str,
    ) -> Result<(), DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let (read_owner, id) = (owner.to_owned(), installation_id.to_owned());
        let generation =
            tokio::task::spawn_blocking(move || store.get_app_installation(&read_owner, &id))
                .await
                .map_err(|_| copy_error("an App copy could not be read"))?
                .map_err(|_| copy_error("an App copy could not be read"))?
                .generation;
        self.copy_request(
            owner,
            installation_id,
            LocalDaemonRequest::UninstallApp(UninstallAppRequest {
                installation_id: installation_id.to_owned(),
                expected_generation: generation.to_string(),
                delete_data: true,
            }),
            "an App copy could not be uninstalled",
        )
        .await
    }

    async fn copy_request(
        &self,
        owner: &str,
        installation_id: &str,
        request: LocalDaemonRequest,
        what: &str,
    ) -> Result<(), DaemonError> {
        self.app_control_response(owner.to_owned(), installation_id.to_owned(), request)
            .await
            .map(|_| ())
            .map_err(|code| app_error(what, code))
    }

    /// The copy's agents use the copies of the Apps their source agents use.
    async fn remap_copy_app_grants(
        &self,
        owner: &str,
        agents: &BTreeMap<String, String>,
        copies: &BTreeMap<String, String>,
    ) -> Result<(), DaemonError> {
        for agent_id in agents.values() {
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            for (source, copy) in copies {
                if !agent.has_extension_grant(ExtensionKind::App, source) {
                    continue;
                }
                if !agent.has_extension_grant(ExtensionKind::App, copy) {
                    self.grant_agent_extension(agent_id, ExtensionGrant::app(copy), owner)
                        .await?;
                }
                self.revoke_agent_extension(agent_id, ExtensionKind::App, source, owner)
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_copy_connections(
        &self,
        owner: &str,
        copy: &AppSetInstallation,
        app: &serde_json::Value,
    ) -> Result<(), DaemonError> {
        let planned = app["connections"].as_array().cloned().unwrap_or_default();
        for connection in &planned {
            let connection_id = text(connection, "connection_id");
            if copy
                .connections
                .iter()
                .any(|granted| granted.connection_id == connection_id)
            {
                continue;
            }
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::GrantAppConnection(GrantAppConnectionRequest {
                    installation_id: copy.installation_id.clone(),
                    generator_id: text(connection, "generator_id").to_owned(),
                    connection_id: connection_id.to_owned(),
                }),
                "a connection could not be shared with an App copy",
            )
            .await?;
        }
        for granted in copy.connections.iter().filter(|granted| {
            !planned
                .iter()
                .any(|connection| text(connection, "connection_id") == granted.connection_id)
        }) {
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::RevokeAppConnection(RevokeAppConnectionRequest {
                    installation_id: copy.installation_id.clone(),
                    connection_id: granted.connection_id.clone(),
                }),
                "a connection could not be revoked from an App copy",
            )
            .await?;
        }
        Ok(())
    }

    async fn apply_copy_automations(
        &self,
        owner: &str,
        copy: &AppSetInstallation,
        app: &serde_json::Value,
        session_id: &str,
        publication_id: &str,
    ) -> Result<(), DaemonError> {
        let planned = app["automations"].as_array().cloned().unwrap_or_default();
        for automation in &planned {
            let automation_id = text(automation, "automation_id");
            let queue_id = text(automation, "queue_id");
            let scheduled = automation["scheduled"].as_bool().unwrap_or_default();
            let delivery_mode = serde_json::from_value(
                automation
                    .get("delivery_mode")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!("queue")),
            )
            .map_err(|_| DaemonError::LocalTransport {
                operation: "copy App automation",
                message: "invalid delivery mode".into(),
            })?;
            let current = copy
                .automations
                .iter()
                .find(|current| current.automation_id == automation_id);
            if current.is_some_and(|current| {
                current.status == AppAutomationStatus::Active
                    && current.session_id == session_id
                    && current.publication_id == publication_id
                    && current.queue_id == queue_id
                    && current.event_name == text(automation, "event_name")
                    && current.scheduled == scheduled
                    && current.delivery_mode == delivery_mode
            }) {
                continue;
            }
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::ConfigureAppAutomation(ConfigureAppAutomationRequest {
                    installation_id: copy.installation_id.clone(),
                    automation_id: automation_id.to_owned(),
                    expected_revision: current.map_or(0, |current| current.revision),
                    event_name: text(automation, "event_name").to_owned(),
                    session_id: session_id.to_owned(),
                    publication_ref: publication_id.to_owned(),
                    queue_ref: Some(queue_id.to_owned()),
                    scheduled,
                    delivery_mode,
                }),
                "an App automation could not be configured for the copy",
            )
            .await?;
        }
        for current in copy.automations.iter().filter(|current| {
            current.status == AppAutomationStatus::Active
                && !planned
                    .iter()
                    .any(|automation| text(automation, "automation_id") == current.automation_id)
        }) {
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::DisableAppAutomation(DisableAppAutomationRequest {
                    installation_id: copy.installation_id.clone(),
                    automation_id: current.automation_id.clone(),
                    expected_revision: current.revision,
                }),
                "an App automation could not be disabled on the copy",
            )
            .await?;
        }
        Ok(())
    }

    /// The copy gets the plan's routes, with the filters of the owner's routes
    /// they were pinned from.
    async fn apply_copy_routes(
        &self,
        owner: &str,
        copy: &AppSetInstallation,
        app: &serde_json::Value,
        set: &[AppSetInstallation],
    ) -> Result<(), DaemonError> {
        let planned = app["inbox_routes"].as_array().cloned().unwrap_or_default();
        let source = text(app, "installation_id");
        let source_routes = set
            .iter()
            .find(|installation| installation.installation_id == source)
            .map(|installation| installation.inbox_routes.as_slice())
            .unwrap_or_default();
        for route in &planned {
            let route_id = text(route, "route_id");
            if copy
                .inbox_routes
                .iter()
                .any(|current| current.route_id == route_id)
            {
                continue;
            }
            let connection = match route["connection"].is_null() {
                true => None,
                false => Some(
                    source_routes
                        .iter()
                        .find(|current| current.route_id == route_id)
                        .and_then(|current| current.connection.clone())
                        .ok_or_else(|| {
                            copy_error(format!(
                                "the owner's inbox route `{route_id}` of App `{source}` no longer exists"
                            ))
                        })?,
                ),
            };
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::CreateAppInboxRoute(CreateAppInboxRouteRequest {
                    installation_id: copy.installation_id.clone(),
                    route_id: route_id.to_owned(),
                    event_name: text(route, "event_name").to_owned(),
                    source_event_type: text(route, "source_event_type").to_owned(),
                    source_event_version: route["source_event_version"]
                        .as_u64()
                        .and_then(|version| u32::try_from(version).ok())
                        .unwrap_or_default(),
                    connection,
                }),
                "an inbox route could not be created for the copy",
            )
            .await?;
        }
        for current in copy.inbox_routes.iter().filter(|current| {
            !planned
                .iter()
                .any(|route| text(route, "route_id") == current.route_id)
        }) {
            self.copy_request(
                owner,
                &copy.installation_id,
                LocalDaemonRequest::RemoveAppInboxRoute(AppInboxRouteRequest {
                    installation_id: copy.installation_id.clone(),
                    route_id: current.route_id.clone(),
                }),
                "an inbox route could not be removed from the copy",
            )
            .await?;
        }
        Ok(())
    }

    /// Handover: an owner's generator-fed route pauses while a deployment
    /// copy's route claims the same event interest, and resumes when none
    /// does (not counting the copies of `leaving`). Only this handover pauses
    /// a route.
    async fn balance_owner_inbox_routes(
        &self,
        owner: &str,
        leaving: Option<&str>,
    ) -> Result<(), DaemonError> {
        let set = self.app_set_for(owner).await?;
        let claimed = set
            .iter()
            .filter(|installation| {
                installation.deployment_id.is_some()
                    && installation.deployment_id.as_deref() != leaving
            })
            .flat_map(|installation| &installation.inbox_routes)
            .filter(|route| route.active)
            .filter_map(|route| {
                interest(
                    route.connection.as_ref()?,
                    &route.source_event_type,
                    route.source_event_version,
                )
            })
            .collect::<BTreeSet<_>>();
        for installation in set
            .iter()
            .filter(|installation| installation.deployment_id.is_none())
        {
            for route in &installation.inbox_routes {
                let Some(connection) = route.connection.as_ref() else {
                    continue;
                };
                let taken_over = interest(
                    connection,
                    &route.source_event_type,
                    route.source_event_version,
                )
                .is_some_and(|key| claimed.contains(&key));
                if route.active == !taken_over {
                    continue;
                }
                match self
                    .inbox(AppInboxOperation::SetRouteActive {
                        owner: owner.to_owned(),
                        installation: installation.installation_id.clone(),
                        route_id: route.route_id.clone(),
                        active: !taken_over,
                    })
                    .await
                {
                    Ok(AppInboxOutcome::Recorded(_)) | Err(AppRequestErrorCode::NotFound) => {}
                    Ok(_) => {}
                    Err(code) => {
                        return Err(app_error(
                            "the owner's inbox route could not be handed over",
                            code,
                        ))
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn fixture_copy_request_id(
    deployment_id: &str,
    release_id: &str,
    app_id: &str,
    attempt: u32,
) -> String {
    format!(
        "{}{attempt}",
        copy_request_prefix(deployment_id, release_id, app_id)
    )
}

#[cfg(test)]
impl KernelRuntimeState {
    /// Applies a deployment's copy of a source publication; the copy's session.
    pub(crate) async fn fixture_ensure_deployment_app_copy(
        &self,
        session_id: &str,
        publication_id: &str,
        deployment_id: &str,
        release_id: &str,
        package_digest: &str,
    ) -> Result<Option<String>, DaemonError> {
        let publication = self
            .owned
            .session_store
            .read()
            .resolve_workflow_publication_ref(session_id, publication_id)?;
        Ok(self
            .ensure_deployment_app_copy(&publication, deployment_id, release_id, package_digest)
            .await?
            .map(|copy| copy.session_id))
    }

    pub(crate) fn fixture_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_store.get_session(session_id)
    }

    pub(crate) fn fixture_session_agents(
        &self,
        session_id: &str,
    ) -> Vec<crate::agent::AgentInstance> {
        self.owned.agent_store.get_session_agents(session_id)
    }

    /// Records deployment metadata (such as a binding) on a publication.
    /// Records `plan` as the App plan of the release with `package_digest`.
    pub(crate) fn fixture_record_release_app_plan(
        &self,
        session_id: &str,
        publication_id: &str,
        package_digest: &str,
        plan: serde_json::Value,
    ) {
        self.owned
            .record_workflow_publication_release(
                session_id,
                publication_id,
                package_digest,
                &[],
                Some(plan),
            )
            .unwrap();
    }

    pub(crate) fn fixture_mark_publication_deployment(
        &self,
        session_id: &str,
        publication_id: &str,
        deployment: serde_json::Value,
    ) {
        self.owned
            .session_store
            .write()
            .mark_workflow_publication_runtime_status(
                session_id,
                publication_id,
                "running",
                None,
                Some(deployment),
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_install_requests_are_stable_and_bounded() {
        let (deployment, release) = ("d".repeat(200), "r".repeat(200));
        let prefix = copy_request_prefix(&deployment, &release, "com.example.state");
        assert_eq!(
            prefix,
            copy_request_prefix(&deployment, &release, "com.example.state")
        );
        assert_ne!(prefix, copy_request_prefix("d", "r2", "com.example.state"));
        assert!(format!("{prefix}{}", u32::MAX).len() <= 128);
    }
}
