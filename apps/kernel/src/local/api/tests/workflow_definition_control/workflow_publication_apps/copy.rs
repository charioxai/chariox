//! P1.20: an App-bound deployment runs as an independent copy of its release's Apps.
use super::*;
use crate::durable_state::app_inbox::AppInboxOperation;
use chariox_app_runtime::app_inbox::{InboxRoute, InboxSource};

const DEPLOYMENT: &str = "deployment-1";
const RELEASE: &str = "release-1";
const PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/pMgE2dD4Y9eL57S6f9+lve+T2A4M0ueD5GmOZfHjkI=\n-----END PUBLIC KEY-----\n";

/// An event trigger fed by the fixture App, whose agent is also granted it,
/// with an owner route on a generator connection; prepared (pinned) and
/// consented for `DEPLOYMENT`/`RELEASE` when `consent`.
struct Deployed {
    graph: PublicationTestGraph,
    publication: crate::session::WorkflowPublicationDefinition,
    digest: String,
}

fn deployed(harness: &LocalRouterTestHarness, label: &str, consent: bool) -> Deployed {
    let graph = create_publication_test_graph(harness, label);
    harness
        .dispatch(LocalDaemonRequest::GrantAgentExtension(
            GrantAgentExtensionRequest {
                workspace_id: None,
                agent_ref: graph.agent_id.clone(),
                kind: crate::local::ExtensionKind::App,
                name: "installed".into(),
                environment: None,
                credential: None,
                max_safety: None,
            },
        ))
        .unwrap();
    let publication = publish(harness, &graph, label, "event_based");
    harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Inject,
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                queue_ref: None,
                scheduled: false,
            },
        ))
        .unwrap();
    // The owner's route as the router would store it after checking the
    // connection with its generator.
    harness
        .with_app(|app| {
            app.durable_state_store()
                .app_inbox(AppInboxOperation::CreateRoute {
                    route: InboxRoute {
                        route_id: "mentions".into(),
                        owner_id: DEFAULT_LOCAL_USER_ID.into(),
                        installation_id: "installed".into(),
                        event_name: "received".into(),
                        source_event_type: "slack.app_mention".into(),
                        source_event_version: 1,
                        active: true,
                        source: Some(InboxSource {
                            generator_id: "slack".into(),
                            connection_id: "connection-1".into(),
                            connection_scope: "team-1".into(),
                            filter_json: r#"{"channel":"social"}"#.into(),
                        }),
                    },
                    now_ms: 1,
                })
        })
        .unwrap();
    let (digest, _) = export(harness, &graph, publication.id()).expect("prepare the package");
    let deployed = Deployed {
        graph,
        publication,
        digest,
    };
    if consent {
        approve(harness, &deployed, RELEASE);
    }
    deployed
}

/// The owner approves deploying `release` with its Apps.
fn approve(harness: &LocalRouterTestHarness, deployed: &Deployed, release: &str) {
    let request_id = format!("consent-{release}");
    let request = || match harness
        .dispatch(LocalDaemonRequest::PrepareDeploymentApps(
            crate::local::PrepareDeploymentAppsRequest {
                session_id: deployed.graph.session_id.clone(),
                request_id: request_id.clone(),
                publication_ref: deployed.publication.id().into(),
                deployment_id: DEPLOYMENT.into(),
                release_id: release.into(),
                package_digest: deployed.digest.clone(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::DeploymentAppsConsent { consent } => consent,
        response => panic!("unexpected response: {response:?}"),
    };
    let asked = request();
    // Protocol 377: App releases approved before are not asked about again.
    if asked.status == crate::local::DeploymentAppsConsentStatus::Approved {
        return;
    }
    harness
        .dispatch(LocalDaemonRequest::RespondToInteraction(
            crate::local::RespondToInteractionRequest {
                session_id: deployed.graph.session_id.clone(),
                interaction_id: asked.interaction_id,
                choice_id: "approve".into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            },
        ))
        .unwrap();
    for _ in 0..200 {
        if request().status == crate::local::DeploymentAppsConsentStatus::Approved {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the consent was not recorded");
}

fn app_set(harness: &LocalRouterTestHarness) -> Vec<crate::local::AppSetInstallation> {
    match harness
        .dispatch(LocalDaemonRequest::GetAppSet(
            crate::local::GetAppSetRequest {},
        ))
        .unwrap()
    {
        LocalDaemonResponse::AppSet { installations, .. } => installations,
        response => panic!("unexpected response: {response:?}"),
    }
}

fn installation<'a>(
    set: &'a [crate::local::AppSetInstallation],
    id: &str,
) -> &'a crate::local::AppSetInstallation {
    set.iter()
        .find(|installation| installation.installation_id == id)
        .unwrap_or_else(|| panic!("installation `{id}` in {set:?}"))
}

/// The (owner, installation) App storages the kernel deleted.
fn deleted_storage(harness: &LocalRouterTestHarness) -> Vec<(String, String)> {
    harness
        .runtime_state()
        .app_control()
        .fixture_app_storage()
        .deleted()
}

fn ensure(
    harness: &LocalRouterTestHarness,
    deployed: &Deployed,
) -> Result<Option<String>, crate::DaemonError> {
    ensure_release(harness, deployed, RELEASE)
}

fn ensure_release(
    harness: &LocalRouterTestHarness,
    deployed: &Deployed,
    release: &str,
) -> Result<Option<String>, crate::DaemonError> {
    let runtime = harness.runtime_state();
    let (session_id, publication_id, digest, release) = (
        deployed.graph.session_id.clone(),
        deployed.publication.id().to_owned(),
        deployed.digest.clone(),
        release.to_owned(),
    );
    let future = async move {
        runtime
            .fixture_ensure_deployment_app_copy(
                &session_id,
                &publication_id,
                DEPLOYMENT,
                &release,
                &digest,
            )
            .await
    };
    // MD-4: a caller must be able to construct this future on the default
    // thread stack; larger test stacks would conceal production regressions.
    assert!(
        std::mem::size_of_val(&future) < 64 * 1024,
        "deployment copy future is too large: {}",
        std::mem::size_of_val(&future)
    );
    harness.block_on_test_task(future)
}

/// Applies the copy while the kernel's pump drives App installs.
fn pumped_ensure(
    harness: &LocalRouterTestHarness,
    deployed: &Deployed,
) -> Result<Option<String>, crate::DaemonError> {
    pumped_ensure_release(harness, deployed, RELEASE)
}

fn pumped_ensure_release(
    harness: &LocalRouterTestHarness,
    deployed: &Deployed,
    release: &str,
) -> Result<Option<String>, crate::DaemonError> {
    let runtime = harness.runtime_state();
    let (session_id, publication_id, digest, release) = (
        deployed.graph.session_id.clone(),
        deployed.publication.id().to_owned(),
        deployed.digest.clone(),
        release.to_owned(),
    );
    let task = harness.spawn_test_task(async move {
        runtime
            .fixture_ensure_deployment_app_copy(
                &session_id,
                &publication_id,
                DEPLOYMENT,
                &release,
                &digest,
            )
            .await
    });
    while !task.is_finished() {
        harness.pump_transport_runtime();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    harness.block_on_test_task(task).unwrap()
}

#[test]
fn a_consented_app_bound_deployment_runs_as_an_independent_copy() {
    let root = temp_root("copy-run");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-run", true);
    // The deployment's own installation of the pinned release (a worker
    // would commit it; the fixture stages and commits it directly).
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });

    let session_id = ensure(&harness, &deployed).unwrap().expect("a copy");
    assert_ne!(session_id, deployed.graph.session_id);
    let copy_session = harness
        .runtime_state()
        .fixture_session(&session_id)
        .unwrap();
    assert!(copy_session.is_hidden());
    // On its owner's kernel the copy works in the source session's workspace.
    let source = harness
        .runtime_state()
        .fixture_session(&deployed.graph.session_id)
        .unwrap();
    assert_eq!(copy_session.workspace_id(), source.workspace_id());
    assert_eq!(copy_session.worktree_id(), source.worktree_id());
    // Its snapshot stays portable, so a session restore keeps the publication.
    let snapshot = copy_session
        .workflow_publication_snapshot(copy_session.workflow_publications()[0].id())
        .expect("the copy's snapshot");
    assert_eq!(
        copy_session.workflow_publications()[0].validate_source_snapshot(snapshot),
        Ok(())
    );
    let copy_publication = &copy_session.workflow_publications()[0];
    assert_eq!(copy_publication.id(), deployed.publication.id());
    assert_eq!(
        copy_publication.kind(),
        crate::session::WORKFLOW_PUBLICATION_KIND_EVENT_BASED,
        "an App-event trigger stays event-based"
    );
    assert_eq!(
        copy_publication.runtime_materialization().unwrap().key,
        format!("deployment:{DEPLOYMENT}:{RELEASE}")
    );
    // The copy's agent uses the copy, not the owner's installation.
    let agents = harness.runtime_state().fixture_session_agents(&session_id);
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].workspace_id(), Some(source.workspace_id()));
    assert_eq!(agents[0].worktree_id(), Some(source.worktree_id()));
    assert!(agents[0].has_extension_grant(crate::extension::ExtensionKind::App, "copy"));
    assert!(!agents[0].has_extension_grant(crate::extension::ExtensionKind::App, "installed"));

    let set = app_set(&harness);
    let copy = installation(&set, "copy");
    assert_eq!(copy.deployment_id.as_deref(), Some(DEPLOYMENT));
    // The automation feeds the copy's publication.
    assert_eq!(copy.automations.len(), 1);
    assert_eq!(copy.automations[0].automation_id, "reminders");
    assert_eq!(
        copy.automations[0].delivery_mode,
        crate::local::NotificationDeliveryMode::Inject
    );
    assert_eq!(copy.automations[0].session_id, session_id);
    assert_eq!(
        copy.automations[0].publication_id,
        deployed.publication.id()
    );
    // The route moved to the copy with the owner's filter; the owner's pauses.
    assert_eq!(copy.inbox_routes.len(), 1);
    let route = &copy.inbox_routes[0];
    assert_eq!(route.route_id, "mentions");
    assert!(route.active);
    let connection = route.connection.as_ref().unwrap();
    assert_eq!(connection.connection_id, "connection-1");
    assert_eq!(connection.filter, serde_json::json!({"channel": "social"}));
    let owner = installation(&set, "installed");
    assert!(
        !owner.inbox_routes[0].active,
        "the owner's route is handed over"
    );
    assert_eq!(owner.automations[0].session_id, deployed.graph.session_id);

    // Applying again (a retried bind, a recovery) changes nothing.
    assert_eq!(
        ensure(&harness, &deployed).unwrap(),
        Some(session_id.clone())
    );
    assert_eq!(app_set(&harness), set);

    // Recovery re-applies what is missing.
    harness
        .dispatch(LocalDaemonRequest::RemoveAppInboxRoute(
            crate::local::AppInboxRouteRequest {
                installation_id: "copy".into(),
                route_id: "mentions".into(),
            },
        ))
        .unwrap();
    assert_eq!(
        ensure(&harness, &deployed).unwrap(),
        Some(session_id.clone())
    );
    let set = app_set(&harness);
    assert!(installation(&set, "copy").inbox_routes[0].active);
    assert!(!installation(&set, "installed").inbox_routes[0].active);

    // Stopping the bound deployment removes the copy and resumes the owner.
    harness.runtime_state().fixture_mark_publication_deployment(
        &deployed.graph.session_id,
        deployed.publication.id(),
        serde_json::json!({
            "kind": "local_runtime",
            "status": "running",
            "binding": {
                "setup_id": "setup-1",
                "operation_key": "deployment-setup:setup-1:runtime",
                "deployment_id": DEPLOYMENT,
                "environment_id": "environment-1",
                "release_id": RELEASE,
                "package_digest": deployed.digest,
                "desired_revision": 1,
                "caller_claims_public_key_pem": PEM,
            },
        }),
    );
    harness
        .dispatch(LocalDaemonRequest::ControlWorkflowPublicationRuntime(
            crate::local::ControlWorkflowPublicationRuntimeRequest {
                session_id: deployed.graph.session_id.clone(),
                publication_ref: deployed.publication.id().into(),
                action: crate::local::WorkflowPublicationRuntimeAction::Stop,
                host: None,
                port: None,
                kernel_url: None,
            },
        ))
        .expect("stop");
    // The stop is durable: a restarted kernel does not serve it again.
    let durable_owner = harness
        .runtime_state()
        .fixture_session(&deployed.graph.session_id)
        .unwrap()
        .host_daemon_id()
        .to_owned();
    let states = harness
        .with_app(|app| {
            app.durable_state_store()
                .load_workflow_hot_states(&durable_owner)
        })
        .unwrap();
    let durable = states
        .iter()
        .find(|(id, _)| *id == deployed.graph.session_id)
        .and_then(|(_, state)| {
            state
                .workflow_publications
                .iter()
                .find(|publication| publication.id() == deployed.publication.id())
        })
        .expect("the durable publication");
    assert_eq!(durable.status(), Some("stopped"));
    let set = app_set(&harness);
    assert!(set
        .iter()
        .all(|installation| installation.installation_id != "copy"));
    assert_eq!(
        deleted_storage(&harness),
        [(DEFAULT_LOCAL_USER_ID.to_owned(), "copy".to_owned())],
        "the copy's data is deleted with it"
    );
    assert!(installation(&set, "installed").inbox_routes[0].active);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while harness.runtime_state().fixture_session(&session_id).is_ok()
        && std::time::Instant::now() < deadline
    {
        harness.pump_transport_runtime();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(harness
        .runtime_state()
        .fixture_session(&session_id)
        .is_err());
    // Starting again is durable before the copy is prepared, so a start that
    // waits for App approvals does not leave the source looking stopped (the
    // reconcile would remove the copies it installed). The start itself may
    // fail here; its intent must not.
    let _ = harness.dispatch(LocalDaemonRequest::ControlWorkflowPublicationRuntime(
        crate::local::ControlWorkflowPublicationRuntimeRequest {
            session_id: deployed.graph.session_id.clone(),
            publication_ref: deployed.publication.id().into(),
            action: crate::local::WorkflowPublicationRuntimeAction::Start,
            host: None,
            port: None,
            kernel_url: None,
        },
    ));
    let states = harness
        .with_app(|app| {
            app.durable_state_store()
                .load_workflow_hot_states(&durable_owner)
        })
        .unwrap();
    let durable = states
        .iter()
        .find(|(id, _)| *id == deployed.graph.session_id)
        .and_then(|(_, state)| {
            state
                .workflow_publications
                .iter()
                .find(|publication| publication.id() == deployed.publication.id())
        })
        .expect("the durable publication");
    let deployment = durable.deployment().expect("the deployment metadata");
    assert!(deployment.get("desired_state").is_none());
    assert_eq!(
        deployment.pointer("/binding/deployment_id"),
        Some(&serde_json::json!(DEPLOYMENT)),
    );
    // A deploy (bind) is an explicit start too: it clears a stop first, so the
    // launch does not yield to a stop that the deploy itself overrides.
    let _ = harness.dispatch(LocalDaemonRequest::ControlWorkflowPublicationRuntime(
        crate::local::ControlWorkflowPublicationRuntimeRequest {
            session_id: deployed.graph.session_id.clone(),
            publication_ref: deployed.publication.id().into(),
            action: crate::local::WorkflowPublicationRuntimeAction::Stop,
            host: None,
            port: None,
            kernel_url: None,
        },
    ));
    let _ = harness.dispatch(LocalDaemonRequest::BindWorkflowPublicationDeployment(
        crate::local::BindWorkflowPublicationDeploymentRequest {
            session_id: deployed.graph.session_id.clone(),
            publication_ref: deployed.publication.id().into(),
            setup_id: "setup-2".into(),
            operation_key: "deployment-setup:setup-2:runtime".into(),
            deployment_id: DEPLOYMENT.into(),
            environment_id: "environment-1".into(),
            release_id: RELEASE.into(),
            package_digest: deployed.digest.clone(),
            desired_revision: 2,
            caller_claims_public_key_pem: PEM.into(),
        },
    ));
    let states = harness
        .with_app(|app| {
            app.durable_state_store()
                .load_workflow_hot_states(&durable_owner)
        })
        .unwrap();
    let rebound = states
        .iter()
        .find(|(id, _)| *id == deployed.graph.session_id)
        .and_then(|(_, state)| {
            state
                .workflow_publications
                .iter()
                .find(|publication| publication.id() == deployed.publication.id())
        })
        .and_then(|publication| publication.deployment())
        .expect("the durable deployment metadata");
    assert!(rebound.get("desired_state").is_none(), "{rebound}");
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn disabling_the_source_publication_removes_its_copy() {
    let root = temp_root("copy-orphan");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-orphan", true);
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });
    let session_id = ensure(&harness, &deployed).unwrap().expect("a copy");
    // An unbound source keeps its copy: a bind may be starting.
    harness.pump_transport_runtime();
    assert!(harness.runtime_state().fixture_session(&session_id).is_ok());
    assert!(!installation(&app_set(&harness), "installed").inbox_routes[0].active);
    assert!(deleted_storage(&harness).is_empty());

    harness
        .dispatch(LocalDaemonRequest::DisableWorkflowPublication(
            crate::local::DisableWorkflowPublicationRequest {
                session_id: deployed.graph.session_id.clone(),
                publication_ref: deployed.publication.id().into(),
            },
        ))
        .expect("disable");
    harness.pump_transport_runtime();
    let set = app_set(&harness);
    assert!(set
        .iter()
        .all(|installation| installation.installation_id != "copy"));
    assert_eq!(
        deleted_storage(&harness),
        [(DEFAULT_LOCAL_USER_ID.to_owned(), "copy".to_owned())]
    );
    assert!(installation(&set, "installed").inbox_routes[0].active);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while harness.runtime_state().fixture_session(&session_id).is_ok()
        && std::time::Instant::now() < deadline
    {
        harness.pump_transport_runtime();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(harness
        .runtime_state()
        .fixture_session(&session_id)
        .is_err());
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_app_bound_bind_without_the_owners_consent_is_refused() {
    let root = temp_root("copy-refused");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-refused", false);
    let error = harness
        .dispatch(LocalDaemonRequest::BindWorkflowPublicationDeployment(
            crate::local::BindWorkflowPublicationDeploymentRequest {
                session_id: deployed.graph.session_id.clone(),
                publication_ref: deployed.publication.id().into(),
                setup_id: "setup-1".into(),
                operation_key: "deployment-setup:setup-1:runtime".into(),
                deployment_id: DEPLOYMENT.into(),
                environment_id: "environment-1".into(),
                release_id: RELEASE.into(),
                package_digest: deployed.digest.clone(),
                desired_revision: 1,
                caller_claims_public_key_pem: PEM.into(),
            },
        ))
        .expect_err("no consent");
    assert!(error.to_string().contains("approve"), "{error}");
    // Nothing was copied and the owner's route still receives.
    let set = app_set(&harness);
    assert_eq!(set.len(), 1);
    assert!(set[0].inbox_routes[0].active);
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_bind_the_kernel_cannot_verify_is_refused_before_the_running_release_stops() {
    let root = temp_root("copy-unverified");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-unverified", true);
    let runtime = harness.runtime_state();
    let (session_id, publication_id) = (
        deployed.graph.session_id.clone(),
        deployed.publication.id().to_owned(),
    );
    // The release that runs now.
    let running = {
        let (runtime, session_id, publication_id) =
            (runtime.clone(), session_id.clone(), publication_id.clone());
        move || {
            let (runtime, session_id, publication_id) =
                (runtime.clone(), session_id.clone(), publication_id.clone());
            async move {
                runtime
                    .fixture_publication_runtime_running(&session_id, &publication_id)
                    .await
            }
        }
    };
    harness.block_on_test_task({
        let (runtime, session_id, publication_id) =
            (runtime.clone(), session_id.clone(), publication_id.clone());
        async move {
            runtime
                .fixture_run_publication_runtime(&session_id, &publication_id)
                .await
        }
    });
    assert!(harness.block_on_test_task(running()));
    // A release this kernel cannot verify (as a rollback to a release whose
    // source changed since) is refused before the running release stops.
    let error = harness
        .dispatch(LocalDaemonRequest::BindWorkflowPublicationDeployment(
            crate::local::BindWorkflowPublicationDeploymentRequest {
                session_id: session_id.clone(),
                publication_ref: publication_id.clone(),
                setup_id: "setup-1".into(),
                operation_key: "deployment-setup:setup-1:runtime".into(),
                deployment_id: DEPLOYMENT.into(),
                environment_id: "environment-1".into(),
                release_id: RELEASE.into(),
                package_digest: format!("sha256:{}", "f".repeat(64)),
                desired_revision: 1,
                caller_claims_public_key_pem: PEM.into(),
            },
        ))
        .expect_err("unverifiable release");
    // Refused by the missing plan, or (once a release without Apps re-exports
    // with none) by the digest check.
    assert!(
        error.to_string().contains("export the release again")
            || error
                .to_string()
                .contains("no longer matches the bound deployment"),
        "{error}"
    );
    assert!(
        harness.block_on_test_task(running()),
        "the running release still runs"
    );
    let publication = harness
        .runtime_state()
        .fixture_session(&session_id)
        .expect("session")
        .workflow_publications()
        .iter()
        .find(|publication| publication.id() == publication_id)
        .cloned()
        .expect("publication");
    assert_ne!(
        publication.status(),
        Some("error"),
        "nothing was marked failed"
    );
    let set = app_set(&harness);
    assert_eq!(set.len(), 1, "nothing was copied");
    assert!(
        set[0].inbox_routes[0].active,
        "the owner's route still receives"
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_copy_install_the_consent_does_not_cover_fails_the_deployment_clearly() {
    let root = temp_root("copy-prompt");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-prompt", true);
    // The fixture's capabilities were never approved by the owner in a prompt,
    // so the consent does not cover the copy's install: the owner is asked.
    let error = pumped_ensure(&harness, &deployed).expect_err("the owner must answer");
    assert!(error.to_string().contains("approval"), "{error}");
    // Deploying again waits for the same prompt; a cancelled (or declined)
    // install is spent, and the next deployment asks anew.
    let attempt = |attempt| {
        crate::runtime::state::fixture_copy_request_id(
            DEPLOYMENT,
            RELEASE,
            "com.example.state",
            attempt,
        )
    };
    let status = |attempt: String| {
        harness.with_app(|app| {
            app.durable_state_store()
                .first_app_install_status(DEFAULT_LOCAL_USER_ID, &attempt)
                .map(|operation| operation.phase)
        })
    };
    assert!(pumped_ensure(&harness, &deployed).is_err());
    assert!(status(attempt(1)).is_err(), "the prompt is still open");
    harness
        .dispatch(LocalDaemonRequest::CancelAppInstallOperation(
            crate::local::AppInstallOperationRequest {
                request_id: attempt(0),
            },
        ))
        .unwrap();
    let error = pumped_ensure(&harness, &deployed).expect_err("asked again");
    assert!(error.to_string().contains("approval"), "{error}");
    assert_eq!(
        status(attempt(1)),
        Ok(crate::durable_state::app_installation_operations::InstallPhase::AwaitingApproval)
    );
    // Spent attempts never run out: each deployment continues from the latest.
    for spent in 1..10 {
        harness
            .dispatch(LocalDaemonRequest::CancelAppInstallOperation(
                crate::local::AppInstallOperationRequest {
                    request_id: attempt(spent),
                },
            ))
            .unwrap();
        let error = pumped_ensure(&harness, &deployed).expect_err("asked again");
        assert!(error.to_string().contains("approval"), "{error}");
    }
    assert_eq!(
        status(attempt(10)),
        Ok(crate::durable_state::app_installation_operations::InstallPhase::AwaitingApproval)
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_release_with_an_older_data_schema_than_the_copy_fails_closed() {
    let root = temp_root("copy-schema");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-schema", true);
    let newer = crate::durable_state::app_state::fixture_release_package("1.1.0", 1, false);
    stage_release(&harness, newer.clone());
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            newer,
        )
    });
    // A route the copy took over before the failure is withdrawn: the copy's
    // runtime does not start, so the owner keeps its occurrences.
    harness
        .with_app(|app| {
            app.durable_state_store()
                .app_inbox(AppInboxOperation::CreateRoute {
                    route: InboxRoute {
                        route_id: "mentions".into(),
                        owner_id: DEFAULT_LOCAL_USER_ID.into(),
                        installation_id: "copy".into(),
                        event_name: "received".into(),
                        source_event_type: "slack.app_mention".into(),
                        source_event_version: 1,
                        active: true,
                        source: Some(InboxSource {
                            generator_id: "slack".into(),
                            connection_id: "connection-1".into(),
                            connection_scope: "team-1".into(),
                            filter_json: r#"{"channel":"social"}"#.into(),
                        }),
                    },
                    now_ms: 2,
                })
        })
        .unwrap();
    let error = ensure(&harness, &deployed).expect_err("a rollback across data schemas");
    assert!(error.to_string().contains("schema version"), "{error}");
    let set = app_set(&harness);
    assert!(installation(&set, "copy").inbox_routes.is_empty());
    assert!(installation(&set, "installed").inbox_routes[0].active);
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Protocol 377: a release whose App has a newer data schema updates the copy
/// in place (the update migrates its data); only an older schema fails closed.
#[test]
fn a_release_with_a_newer_data_schema_updates_the_copy() {
    let root = temp_root("copy-migrate");
    let harness = harness_with_app(&root);
    let first = deployed(&harness, "copy-migrate", true);
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });
    // The owner updates the App to a release with a newer data schema, and
    // the next release of the deployment packages it.
    let newer = crate::durable_state::app_state::fixture_inbox_package_version("1.1.0", 1);
    stage_release(&harness, newer.clone());
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_update_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "installed",
            newer,
        )
    });
    let (digest, files) =
        export(&harness, &first.graph, first.publication.id()).expect("the next release");
    assert_eq!(
        package_json_file(&files, "apps.json")["apps"][0]["schema_version"],
        1
    );
    let next = Deployed {
        graph: first.graph,
        publication: first.publication,
        digest,
    };
    approve(&harness, &next, "release-2");
    // The copy's update is begun rather than refused for its schema; this
    // harness runs no App worker, so the update itself cannot complete here:
    // it fails, or (under load) is still running at the install deadline.
    let error = pumped_ensure_release(&harness, &next, "release-2").expect_err("no App worker");
    let error = error.to_string();
    assert!(!error.contains("schema version"), "{error}");
    assert!(
        error.contains("could not be installed for the deployment")
            || error.contains("did not finish installing for the deployment"),
        "{error}"
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Rollback: binding another release of the deployment moves the copy to that
/// release's session and plan, keeping the copy's installation (and its data)
/// when the release pins the same App release.
#[test]
fn binding_another_release_re_applies_its_plan_on_the_same_copy() {
    let root = temp_root("copy-release");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-release", true);
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });
    let first = ensure(&harness, &deployed).unwrap().unwrap();
    // Another release needs its own consent; with the same App releases it
    // is approved without asking again (protocol 377).
    let refused = ensure_release(&harness, &deployed, "release-2").expect_err("no consent");
    assert!(refused.to_string().contains("approve"), "{refused}");
    approve(&harness, &deployed, "release-2");
    let second = ensure_release(&harness, &deployed, "release-2")
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert!(harness.runtime_state().fixture_session(&first).is_err());
    let copy_publication = harness.runtime_state().fixture_session(&second).unwrap();
    assert_eq!(
        copy_publication.workflow_publications()[0]
            .runtime_materialization()
            .unwrap()
            .key,
        format!("deployment:{DEPLOYMENT}:release-2")
    );
    let set = app_set(&harness);
    let copy = installation(&set, "copy");
    assert_eq!(copy.automations[0].session_id, second);
    assert!(copy.inbox_routes[0].active);
    assert!(!installation(&set, "installed").inbox_routes[0].active);
    assert!(harness.runtime_state().fixture_session_agents(&second)[0]
        .has_extension_grant(crate::extension::ExtensionKind::App, "copy"));
    // Rolling back applies release 1 again.
    assert_ne!(ensure(&harness, &deployed).unwrap().unwrap(), second);
    assert!(harness.runtime_state().fixture_session(&second).is_err());
    // The same copy served both releases: its data was never deleted.
    assert!(deleted_storage(&harness).is_empty());
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// A release whose plan names no App (its last App was removed) runs from the
/// source: binding it removes the deployment's copy and resumes the owner.
#[test]
fn a_release_without_apps_removes_the_copy_and_resumes_the_owner() {
    let root = temp_root("copy-no-apps");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-no-apps", true);
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });
    let copy_session = ensure(&harness, &deployed).unwrap().expect("a copy");
    let set = app_set(&harness);
    assert!(installation(&set, "copy").inbox_routes[0].active);
    assert!(!installation(&set, "installed").inbox_routes[0].active);

    let empty = format!("sha256:{}", "e".repeat(64));
    harness.runtime_state().fixture_record_release_app_plan(
        &deployed.graph.session_id,
        deployed.publication.id(),
        &empty,
        serde_json::json!({"schema": "chariox.publication-apps.v1", "apps": []}),
    );
    let next = Deployed {
        graph: deployed.graph,
        publication: deployed.publication,
        digest: empty,
    };
    assert_eq!(ensure_release(&harness, &next, "release-2").unwrap(), None);
    let set = app_set(&harness);
    assert!(
        set.iter()
            .all(|installation| installation.deployment_id.as_deref() != Some(DEPLOYMENT)),
        "the deployment's copy installations are gone: {set:?}"
    );
    assert!(
        installation(&set, "installed").inbox_routes[0].active,
        "the owner's route resumes"
    );
    assert!(harness
        .runtime_state()
        .fixture_session(&copy_session)
        .is_err());
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_stop_that_lands_while_a_runtime_starts_wins() {
    let root = temp_root("copy-stop-race");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-stop-race", true);
    let (session_id, publication_id) = (
        deployed.graph.session_id.clone(),
        deployed.publication.id().to_owned(),
    );
    harness.runtime_state().fixture_mark_publication_deployment(
        &session_id,
        &publication_id,
        serde_json::json!({
            "kind": "local_runtime",
            "status": "starting",
            "binding": {
                "setup_id": "setup-1",
                "operation_key": "deployment-setup:setup-1:runtime",
                "deployment_id": DEPLOYMENT,
                "environment_id": "environment-1",
                "release_id": RELEASE,
                "package_digest": deployed.digest,
                "desired_revision": 1,
                "caller_claims_public_key_pem": PEM,
            },
        }),
    );
    let register = || {
        let (runtime, session_id, publication_id) = (
            harness.runtime_state(),
            session_id.clone(),
            publication_id.clone(),
        );
        harness.block_on_test_task(async move {
            let outcome = runtime
                .fixture_register_launched_runtime(&session_id, &publication_id)
                .await;
            let running = runtime
                .fixture_publication_runtime_running(&session_id, &publication_id)
                .await;
            (outcome, running)
        })
    };
    let alive = |pid: u32| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    };
    // Without a stop, a launched gateway is registered and runs.
    let ((outcome, first), running) = register();
    outcome.expect("registered");
    assert!(running && alive(first));
    // The owner stops the deployment; its next gateway is still starting
    // (not registered yet) when the stop lands.
    harness
        .dispatch(LocalDaemonRequest::ControlWorkflowPublicationRuntime(
            crate::local::ControlWorkflowPublicationRuntimeRequest {
                session_id: session_id.clone(),
                publication_ref: publication_id.clone(),
                action: crate::local::WorkflowPublicationRuntimeAction::Stop,
                host: None,
                port: None,
                kernel_url: None,
            },
        ))
        .expect("stop");
    let ((outcome, pid), running) = register();
    let error = outcome.expect_err("the stop wins");
    assert!(
        error
            .to_string()
            .contains("stopped while its runtime was starting"),
        "{error}"
    );
    assert!(!running, "no runtime is registered");
    assert!(!alive(pid), "the launched gateway is gone");
    assert!(!alive(first), "the stop stopped the running gateway");
    // The stop is durable: a restarted kernel does not serve it again.
    let durable_owner = harness
        .runtime_state()
        .fixture_session(&session_id)
        .unwrap()
        .host_daemon_id()
        .to_owned();
    let states = harness
        .with_app(|app| {
            app.durable_state_store()
                .load_workflow_hot_states(&durable_owner)
        })
        .unwrap();
    let durable = states
        .iter()
        .find(|(id, _)| *id == session_id)
        .and_then(|(_, state)| {
            state
                .workflow_publications
                .iter()
                .find(|publication| publication.id() == publication_id)
        })
        .and_then(|publication| publication.deployment())
        .expect("the durable deployment metadata");
    assert_eq!(
        durable.get("desired_state"),
        Some(&serde_json::json!("stopped")),
        "{durable}"
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Rolling back to a release exported before the publication used any App
/// removes the later release's copy and resumes the owner.
#[test]
fn a_rollback_to_a_release_without_apps_removes_the_copy() {
    let root = temp_root("copy-rollback-no-apps");
    let harness = harness_with_app(&root);
    let deployed = deployed(&harness, "copy-rollback-no-apps", true);
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_copy_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "copy",
            DEPLOYMENT,
            crate::durable_state::app_state::fixture_inbox_package(),
        )
    });
    let copy_session = ensure(&harness, &deployed).unwrap().expect("a copy");
    assert!(!installation(&app_set(&harness), "installed").inbox_routes[0].active);
    // A release with no recorded plan while later releases record theirs.
    let earlier = Deployed {
        graph: deployed.graph,
        publication: deployed.publication,
        digest: format!("sha256:{}", "d".repeat(64)),
    };
    assert_eq!(
        ensure_release(&harness, &earlier, "release-0").unwrap(),
        None
    );
    let set = app_set(&harness);
    assert!(
        set.iter()
            .all(|installation| installation.deployment_id.as_deref() != Some(DEPLOYMENT)),
        "the copy's installations are gone: {set:?}"
    );
    assert!(installation(&set, "installed").inbox_routes[0].active);
    assert!(harness
        .runtime_state()
        .fixture_session(&copy_session)
        .is_err());
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}
