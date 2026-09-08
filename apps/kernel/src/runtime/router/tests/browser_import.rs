use super::*;
use serde_json::{json, Value};

#[test]
fn browser_import_consent_requires_verified_owner_and_unchanged_selection() {
    run_consent_round_trip(None);
}

#[test]
fn browser_import_destination_claim_requires_verified_owner() {
    run_consent_round_trip(Some(false));
}

#[test]
fn browser_import_failed_cleanup_retains_admission_block() {
    run_consent_round_trip(Some(true));
}

fn run_consent_round_trip(cleanup_failure: Option<bool>) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(consent_round_trip(cleanup_failure));
        })
        .unwrap()
        .join()
        .unwrap();
}

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn consent_round_trip(cleanup_failure: Option<bool>) {
    let config = DaemonConfig::for_tests();
    let database_path = config.user_config.state.path.clone().unwrap();
    let _scratch = Scratch(
        std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
            .parent()
            .unwrap()
            .to_path_buf(),
    );
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let session = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("browser-import", "worktree"))
        .unwrap()
        .0;
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "import-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let state = &router.runtime_state;
    state
        .start_room_environment(
            session.id(),
            crate::session::CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
        )
        .unwrap();
    state
        .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
    let environment = state
        .reconcile_room_environment_controller_tabs(
            session.id(),
            vec![crate::session::EnvironmentTabObservation {
                runtime_target_id: "target-1".into(),
                document_id: "doc-1".into(),
                url: "https://example.com/".into(),
                title: "fixture".into(),
            }],
            Some("target-1"),
        )
        .unwrap();
    let selection = json!({
        "session_id": session.id(), "attachment_id": attachment.id(),
        "environment_id": environment.environment_id,
        "runtime_generation": environment.runtime_generation,
        "tab_id": environment.tabs[0].tab_id,
        "document_revision": environment.tabs[0].document_revision,
        "source_store_id": "0", "domains": ["example.com"],
        "partition_sites": [], "overwrite": false
    });
    let caller = KernelCaller {
        caller_id: "import-client".into(),
        caller_kind: KernelCallerKind::RemoteClient,
        user_id: Some(DEFAULT_LOCAL_USER_ID.into()),
        client_id: Some("import-client".into()),
        machine_id: None,
        realm_id: Some("test-realm".into()),
        public_key_thumbprint: Some("test-client-key".into()),
        metaagent_id: None,
    };
    let prepare = json!({"PrepareBrowserImport": {"selection": selection}});
    // Seed durable state directly to model a prior executor/kernel restart.
    let database = rusqlite::Connection::open(&database_path).unwrap();
    database
        .execute(
            "INSERT INTO durable_browser_import VALUES (?1, 'prior-request', ?2, ?3, 1)",
            rusqlite::params![
                environment.environment_id,
                DEFAULT_LOCAL_USER_ID,
                session.id()
            ],
        )
        .unwrap();
    assert!(
        dispatch(&router, &caller, prepare.clone()).await.is_err(),
        "durable recovery state must block new consent"
    );
    let action = || {
        crate::session::EnvironmentActionRequest::computer_mutation(
            "fixture-actor",
            environment.runtime_generation,
            "click",
            None,
        )
    };
    assert_eq!(
        state
            .submit_room_environment_action(session.id(), action())
            .unwrap_err(),
        crate::session::EnvironmentError::ImportRecoveryRequired
    );
    database
        .execute(
            "UPDATE durable_browser_import SET recovery_required = 0",
            [],
        )
        .unwrap();
    assert!(
        dispatch(&router, &caller, prepare.clone()).await.is_err(),
        "verified recovery still blocks until journal cleanup"
    );
    assert_eq!(
        state
            .submit_room_environment_action(session.id(), action())
            .unwrap_err(),
        crate::session::EnvironmentError::ImportRecoveryRequired
    );
    database
        .execute("DELETE FROM durable_browser_import", [])
        .unwrap();
    drop(database);
    assert!(state
        .ensure_no_pending_environment_import(session.id())
        .is_ok());
    let mut unverified = caller.clone();
    unverified.public_key_thumbprint = None;
    assert!(dispatch(&router, &unverified, prepare.clone())
        .await
        .is_err());
    let response = dispatch(&router, &caller, prepare.clone()).await.unwrap();
    let id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    assert_eq!(response["BrowserImportConsent"]["status"], "prepared");
    assert!(
        dispatch(&router, &caller, prepare.clone()).await.is_err(),
        "only one pending import per Environment"
    );
    let approve = json!({"ApproveBrowserImport": {"request_id": id, "selection": selection}});
    let claim = json!({"ClaimBrowserImportSource": {"request_id": id, "selection": selection}});
    let authorize =
        json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": selection}});
    assert!(
        dispatch(&router, &caller, claim.clone()).await.is_err(),
        "source read requires approval"
    );
    let mut rotated = caller.clone();
    rotated.public_key_thumbprint = Some("replacement-key".into());
    assert!(dispatch(&router, &rotated, approve.clone()).await.is_err());
    rotated = caller.clone();
    rotated.realm_id = Some("other-realm".into());
    assert!(dispatch(&router, &rotated, approve.clone()).await.is_err());
    let mut wrong_client = caller.clone();
    wrong_client.client_id = Some("other-client".into());
    assert!(dispatch(&router, &wrong_client, approve.clone())
        .await
        .is_err());
    let mut wrong_scope = approve.clone();
    wrong_scope["ApproveBrowserImport"]["selection"]["overwrite"] = json!(true);
    assert!(dispatch(&router, &caller, wrong_scope).await.is_err());
    let mut agent = caller.clone();
    agent.caller_kind = KernelCallerKind::Metaagent;
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    agent.caller_kind = KernelCallerKind::HostedService;
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    agent = caller.clone();
    agent.user_id = Some("other-user".into());
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    for (field, value) in [
        ("source_store_id", json!("other-store")),
        ("domains", json!(["other.example"])),
        ("partition_sites", json!(["https://other.example"])),
        ("runtime_generation", json!(99)),
        ("environment_id", json!("wrong-environment")),
        ("tab_id", json!("wrong-tab")),
    ] {
        let mut changed = approve.clone();
        changed["ApproveBrowserImport"]["selection"][field] = value;
        assert!(
            dispatch(&router, &caller, changed).await.is_err(),
            "changed {field} cannot reuse consent"
        );
    }
    assert_eq!(
        dispatch(&router, &caller, approve.clone()).await.unwrap()["BrowserImportConsent"]
            ["status"],
        "approved"
    );
    assert!(
        dispatch(&router, &caller, approve).await.is_err(),
        "approval cannot replay"
    );
    assert!(
        dispatch(&router, &caller, authorize.clone()).await.is_err(),
        "approval alone is not a claimed source read"
    );
    assert_eq!(
        dispatch(&router, &caller, claim.clone()).await.unwrap()["BrowserImportConsent"]["status"],
        "source_claimed"
    );
    assert!(
        dispatch(&router, &caller, claim).await.is_err(),
        "source claim cannot replay"
    );
    assert_eq!(
        dispatch(&router, &caller, authorize.clone()).await.unwrap()["BrowserImportConsent"]
            ["status"],
        "source_authorized"
    );
    assert!(
        dispatch(&router, &rotated, authorize.clone())
            .await
            .is_err(),
        "realm changes invalidate an active source read"
    );
    for (field, value) in [
        ("source_store_id", json!("other-store")),
        ("domains", json!(["other.example"])),
        ("partition_sites", json!(["https://other.example"])),
        ("overwrite", json!(true)),
        ("runtime_generation", json!(99)),
        ("document_revision", json!(99)),
    ] {
        let mut changed = authorize.clone();
        changed["AuthorizeBrowserImportSource"]["selection"][field] = value;
        assert!(
            dispatch(&router, &caller, changed).await.is_err(),
            "active source read rejects changed {field}"
        );
    }
    let mut denied_caller = caller.clone();
    denied_caller.public_key_thumbprint = Some("replacement-key".into());
    assert!(dispatch(&router, &denied_caller, authorize.clone())
        .await
        .is_err());
    denied_caller = caller.clone();
    denied_caller.caller_kind = KernelCallerKind::Metaagent;
    assert!(dispatch(&router, &denied_caller, authorize.clone())
        .await
        .is_err());
    if let Some(cleanup_failure) = cleanup_failure {
        let destination_selection = serde_json::from_value(selection.clone()).unwrap();
        let destination_request: LocalDaemonRequest =
            serde_json::from_value(authorize.clone()).unwrap();
        let destination_command = |identity: KernelCaller| {
            KernelCommand::from_local_request_with_caller(
                "destination-test",
                KernelCommandSource::RelayClient,
                identity,
                None,
                None,
                &destination_request,
            )
        };
        assert!(state
            .claim_browser_import_destination(
                &destination_command(denied_caller.clone()),
                &destination_selection,
                id,
            )
            .await
            .is_err());
        let destination_guard = state
            .claim_browser_import_destination(
                &destination_command(caller.clone()),
                &destination_selection,
                id,
            )
            .await
            .unwrap();
        assert!(state
            .ensure_no_pending_environment_import(session.id())
            .is_err());
        let completion = destination_guard
            .complete_after_verification(async {
                assert!(state
                    .ensure_no_pending_environment_import(session.id())
                    .is_err());
                let database = rusqlite::Connection::open(&database_path).unwrap();
                let recovery_required: bool = database.query_row(
                "SELECT recovery_required FROM durable_browser_import WHERE environment_id = ?1",
                [&environment.environment_id], |row| row.get(0),
            ).unwrap();
                assert!(
                    !recovery_required,
                    "durable verification must precede journal cleanup"
                );
                if cleanup_failure {
                    Err(DaemonError::LocalTransport {
                        operation: "fixture cleanup",
                        message: "synthetic private storage detail".into(),
                    })
                } else {
                    Ok(())
                }
            })
            .await;
        if cleanup_failure {
            let error = completion.unwrap_err();
            assert!(!format!("{error:?}").contains("synthetic private storage detail"));
            assert!(state
                .ensure_no_pending_environment_import(session.id())
                .is_err());
            assert!(dispatch(&router, &caller, prepare).await.is_err());
            return;
        }
        completion.unwrap();
        assert!(state
            .ensure_no_pending_environment_import(session.id())
            .is_ok());
        assert!(
            dispatch(&router, &caller, prepare).await.is_ok(),
            "completion must release the in-memory consent claim too"
        );
        return;
    }
    let cancel = json!({"CancelBrowserImport": {"session_id": session.id(), "attachment_id": attachment.id(), "request_id": id}});
    assert_eq!(
        dispatch(&router, &caller, cancel).await.unwrap()["BrowserImportConsent"]["status"],
        "cancelled"
    );
    assert!(
        dispatch(&router, &caller, authorize).await.is_err(),
        "cancelled source read must discard its result"
    );
    let response = dispatch(&router, &caller, prepare).await.unwrap();
    let next_id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    dispatch(
        &router,
        &caller,
        json!({"ApproveBrowserImport": {"request_id": next_id, "selection": selection}}),
    )
    .await
    .unwrap();
    dispatch(
        &router,
        &caller,
        json!({"ClaimBrowserImportSource": {"request_id": next_id, "selection": selection}}),
    )
    .await
    .unwrap();
    state
        .reconcile_room_environment_controller_tabs(
            session.id(),
            vec![crate::session::EnvironmentTabObservation {
                runtime_target_id: "target-1".into(),
                document_id: "doc-2".into(),
                url: "https://example.com/next".into(),
                title: "next".into(),
            }],
            Some("target-1"),
        )
        .unwrap();
    assert!(
        dispatch(
            &router,
            &caller,
            json!({"AuthorizeBrowserImportSource": {"request_id": next_id, "selection": selection}})
        )
        .await
        .is_err(),
        "navigation invalidates an active source read"
    );
    let cancel = json!({"CancelBrowserImport": {"session_id": session.id(), "attachment_id": attachment.id(), "request_id": next_id}});
    assert!(
        dispatch(&router, &caller, cancel).await.is_ok(),
        "navigation must not prevent cancellation"
    );
    let mut current = selection.clone();
    current["document_revision"] =
        json!(state.room_environment_snapshot(session.id()).unwrap().tabs[0].document_revision);
    let local = KernelCaller::for_source(&KernelCommandSource::LocalIpc);
    let request = json!({"PrepareBrowserImport": {"selection": current}});
    let response = dispatch_source(&router, &local, request, KernelCommandSource::LocalIpc)
        .await
        .unwrap();
    let local_id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    let request = json!({"ApproveBrowserImport": {"request_id": local_id, "selection": current}});
    assert!(
        dispatch(&router, &caller, request.clone()).await.is_err(),
        "relay cannot approve local consent"
    );
    assert!(
        dispatch_source(&router, &local, request, KernelCommandSource::LocalIpc)
            .await
            .is_ok()
    );
}

async fn dispatch(
    router: &CommandRouter,
    caller: &KernelCaller,
    value: Value,
) -> Result<Value, DaemonError> {
    dispatch_source(router, caller, value, KernelCommandSource::RelayClient).await
}

async fn dispatch_source(
    router: &CommandRouter,
    caller: &KernelCaller,
    value: Value,
    source: KernelCommandSource,
) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest =
        serde_json::from_value(value).expect("browser import request must decode");
    let command = KernelCommand::from_local_request_with_caller(
        "browser-import-test",
        source,
        caller.clone(),
        None,
        None,
        &request,
    );
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}
