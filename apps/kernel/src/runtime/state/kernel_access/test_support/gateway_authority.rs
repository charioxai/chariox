//! EA2-10: the origin and the target each make their own half of a forwarded
//! request's decision, and neither needs the other's state.
use super::*;
use crate::runtime::kernel_access::Refusal;

fn state_with_live_grant() -> (KernelRuntimeState, String) {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state();
    let holder = process::inspect(std::process::id()).unwrap().0;
    state.owned.kernel_access.lock().unwrap().grants.insert(
        "gateway-grant".into(),
        Grant {
            summary: KernelAccessGrant {
                grant_id: "gateway-grant".into(),
                owner_user_id: "local".into(),
                holder_pid: holder.pid,
                holder_executable: holder.executable.clone(),
                holder_harness: None,
                term: KernelAccessTerm::UntilHolderExits,
                expires_at_ms: None,
            },
            holder,
            clock: None,
        },
    );
    (state, "gateway-grant".into())
}

fn answer(passkey: bool) -> LocalDaemonRequest {
    let mut value = serde_json::json!({
        "session_id": "remote-room", "interaction_id": "remote-question", "choice_id": "yes",
    });
    if passkey {
        value["passkey"] = serde_json::json!("typed-by-the-agent");
    }
    LocalDaemonRequest::RespondToInteraction(serde_json::from_value(value).unwrap())
}

fn refused_shapes() -> Vec<LocalDaemonRequest> {
    vec![
        answer(true),
        LocalDaemonRequest::ConfigureRelay(crate::local::ConfigureRelayRequest {
            relay_url: Some("wss://elsewhere.invalid".into()),
            relay_token: None,
        }),
        LocalDaemonRequest::ResolveKernelClientConnection(
            crate::local::ResolveKernelClientConnectionRequest {
                kernel_ref: "other".into(),
                machine_ref: None,
                client_id: None,
                session_id: None,
                public_key_thumbprint: None,
            },
        ),
        LocalDaemonRequest::RequestKernelAccess(crate::local::RequestKernelAccessRequest {
            holder_pid: 2,
            term: None,
        }),
        LocalDaemonRequest::ListCredentials(crate::local::ListCredentialsRequest),
    ]
}

#[tokio::test]
async fn the_origin_checks_grant_and_shape_and_leaves_the_remote_interaction_to_the_target() {
    let (state, grant) = state_with_live_grant();
    // A routine question owned by the target is not this kernel's to classify.
    state
        .authorize_gateway_request(&grant, &answer(false))
        .expect("routine remote answer is admitted at the origin");
    for request in refused_shapes() {
        let error = state
            .authorize_gateway_request(&grant, &request)
            .expect_err("human-only shape must be refused at the origin");
        assert_eq!(Refusal::of(&error), Refusal::HumanOnly, "{request:?}");
    }
    assert!(state
        .authorize_gateway_request("revoked-grant", &answer(false))
        .is_err());
    assert!(state
        .authorize_gateway_request("sudo:turn", &answer(false))
        .is_err());
}

#[tokio::test]
async fn the_target_refuses_the_human_only_set_without_knowing_the_origin_grant() {
    let (state, _) = state_with_live_grant();
    let on_behalf_of = crate::local::OnBehalfOf {
        origin_kernel: "origin-kernel".into(),
        origin_name: None,
        grant_id: "grant-that-lives-only-at-the-origin".into(),
        harness: "claude".into(),
    };
    state
        .authorize_on_behalf_of_request(&on_behalf_of, &answer(false))
        .expect("routine answer is admitted at the target");
    for request in refused_shapes() {
        let error = state
            .authorize_on_behalf_of_request(&on_behalf_of, &request)
            .expect_err("human-only shape must be refused at the target");
        assert_eq!(Refusal::of(&error), Refusal::HumanOnly, "{request:?}");
    }
}

fn gateway_origin(grant: String) -> crate::local::OnBehalfOf {
    crate::local::OnBehalfOf {
        origin_kernel: "origin-kernel".into(),
        origin_name: None,
        grant_id: grant,
        harness: "test".into(),
    }
}

fn gateway_audit_rows(state: &KernelRuntimeState) -> i64 {
    let store = &state.owned.durable_state_store;
    // The writer commits in queue order, so a waited append settles the audits before it.
    store
        .append_event("test.barrier", None, serde_json::json!({}))
        .unwrap();
    rusqlite::Connection::open(store.path())
        .unwrap()
        .query_row(
            "SELECT count(*) FROM durable_state_events WHERE kind='kernel_gateway.request'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

// MP-08 / MP-09 / MP-10 / MP-11: status, list and state polls through the gateway leave
// no audit row; a state change leaves one.
#[tokio::test(flavor = "multi_thread")]
async fn the_target_audits_state_changes_and_skips_polls() {
    let (state, grant) = state_with_live_grant();
    let origin = gateway_origin(grant);
    for poll in [
        LocalDaemonRequest::RelayStatus(crate::local::RelayStatusRequest),
        LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest),
        LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
            session_id: "remote-room".into(),
        }),
    ] {
        state.audit_gateway_request(&origin, &poll);
    }
    assert_eq!(gateway_audit_rows(&state), 0);
    state.audit_gateway_request(&origin, &answer(false));
    assert_eq!(gateway_audit_rows(&state), 1);
}

// MP-08 / MP-09 / MP-10 / MP-11: a gateway request does not wait for its audit row.
#[tokio::test(flavor = "multi_thread")]
async fn the_target_audit_does_not_hold_the_request() {
    let (state, grant) = state_with_live_grant();
    let origin = gateway_origin(grant);
    let metrics = crate::runtime::request_metrics::RequestMetrics::default();
    metrics
        .observe("gateway".into(), async {
            state.audit_gateway_request(&origin, &answer(false));
            Ok(LocalDaemonResponse::CloudRelayLoggedOut)
        })
        .await
        .unwrap();
    assert_eq!(metrics.snapshot()[0].durable_wait_us.max, 0);
    assert_eq!(gateway_audit_rows(&state), 1);
}
