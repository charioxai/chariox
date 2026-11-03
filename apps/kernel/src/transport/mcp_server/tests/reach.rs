//! MP-08 / MP-09 / MP-10 / MP-11 reach: the owner's Room agents act on other rooms and
//! kernels through `chariox-reach`; guests' agents do not; /sudo adds what it adds at home.
use super::*;

struct Reach {
    router: Arc<CommandRouter>,
    app: Arc<Mutex<DaemonApp>>,
    session_id: String,
    owner_token: String,
    guest_token: String,
    other_session_id: String,
}

const OWNER_TOKEN: &str = "reach-owner-token";
const GUEST_TOKEN: &str = "reach-guest-token";

/// A live run of `agent_id` that answers to `token`, as a running turn would.
fn launch(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
    token: &str,
) -> String {
    super::permission_prompt::insert_claude_run(
        app,
        session_id,
        agent_id,
        run_id,
        token,
        crate::provider::ProviderClientInterface::Chariox,
    );
    token.to_string()
}

fn fixture() -> Reach {
    let worktree = crate::test_support::TestWorktree::new("reach-worktree");
    let other = crate::test_support::TestWorktree::new("reach-other-worktree");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let (other_session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(other.session_request())
        .expect("second room should be created");
    let guest = {
        let mut sessions = app.sessions_mut();
        let mut agents = app.agents_mut();
        agents
            .create_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("guest-agent")
                    .with_owner_user_id("a-guest"),
                &mut sessions,
            )
            .expect("guest agent should be created")
    };
    let owner_token = launch(
        &mut app,
        session.id(),
        agent.id(),
        "reach-run-owner",
        OWNER_TOKEN,
    );
    let guest_token = launch(
        &mut app,
        session.id(),
        guest.id(),
        "reach-run-guest",
        GUEST_TOKEN,
    );
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 8));
    Reach {
        router,
        app,
        session_id: session.id().to_string(),
        owner_token,
        guest_token,
        other_session_id: other_session.id().to_string(),
    }
}

async fn call(reach: &Reach, token: &str, kernel: Option<&str>, request: Value) -> (bool, Value) {
    let mut arguments = serde_json::json!({"request": request});
    if let Some(kernel) = kernel {
        arguments["kernel"] = kernel.into();
    }
    let result = reach
        .router
        .dispatch_reach_tool_call(token, "request", arguments)
        .await
        .expect("reach call should return a tool result");
    (result.ok, result.payload)
}

fn code(payload: &Value) -> &str {
    payload["code"].as_str().unwrap_or_default()
}

#[test]
fn reach_need_follows_the_agents_own_room_authority() {
    use crate::local::LocalDaemonRequest as R;
    use crate::runtime::state::reach::{reach_need, ReachNeed};
    let parse = |value: Value| serde_json::from_value::<R>(value).expect("request parses");
    for request in [
        serde_json::json!({"ListSessions": null}),
        serde_json::json!({"ListGatewayKernels": {}}),
    ] {
        assert_eq!(reach_need(&parse(request)), ReachNeed::Room);
    }
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"AliasSession": {"session_id": "s", "alias": "a"}})
        )),
        ReachNeed::Sudo,
        "renaming a room needs /sudo in the agent's own room"
    );
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"DeleteSession": {"session_ref": "s"}})
        )),
        ReachNeed::Excluded
    );
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"ConfigureRelay": {"relay_url": "wss://x"}})
        )),
        ReachNeed::HumanOnly
    );
}

#[tokio::test]
async fn the_owners_agent_gets_the_reach_tools_and_a_guests_agent_gets_none() {
    let reach = fixture();
    let owner = reach
        .router
        .reach_tool_specs_for_auth_token(&reach.owner_token);
    let names = owner
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["kernels", "request"]);
    assert!(
        reach
            .router
            .reach_tool_specs_for_auth_token(&reach.guest_token)
            .is_empty(),
        "a guest's agent never reaches"
    );
    let (ok, payload) = call(
        &reach,
        &reach.guest_token,
        None,
        serde_json::json!({"ListSessions": null}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "reach_not_available");
    let (ok, payload) = call(
        &reach,
        "no-such-token",
        None,
        serde_json::json!({"ListSessions": null}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "reach_not_available");
}

#[tokio::test]
async fn reach_lists_every_room_without_sudo_and_refuses_what_needs_sudo() {
    let reach = fixture();
    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        None,
        serde_json::json!({"ListSessions": null}),
    )
    .await;
    assert!(ok, "{payload}");
    let text = payload.to_string();
    assert!(text.contains(&reach.session_id) && text.contains(&reach.other_session_id));

    // The same request kinds at home need /sudo, so elsewhere they do too.
    let alias = serde_json::json!({"AliasSession": {
        "session_id": reach.other_session_id, "alias": "renamed-by-reach"}});
    let (ok, payload) = call(&reach, &reach.owner_token, None, alias.clone()).await;
    assert!(!ok);
    assert_eq!(code(&payload), "reach_requires_sudo");
    let (ok, payload) = call(&reach, &reach.owner_token, Some("kernel-b"), alias).await;
    assert!(!ok);
    assert_eq!(code(&payload), "reach_requires_sudo");

    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        None,
        serde_json::json!({"DeleteSession": {"session_ref": reach.other_session_id}}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "excluded_for_agents");
    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        Some("kernel-b"),
        serde_json::json!({"ConfigureRelay": {"relay_url": "wss://x"}}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "human_only");
}

#[tokio::test]
async fn the_gateway_names_the_agent_and_admits_only_room_requests_for_it() {
    let reach = fixture();
    let state = reach.router.runtime_state();
    let caller = state
        .reach_for_auth_token(&reach.owner_token)
        .expect("the owner's agent reaches");
    assert!(caller.id.starts_with("reach:"));
    assert!(
        caller.harness.ends_with(" (Chariox agent)") && caller.harness.contains('/'),
        "{}",
        caller.harness
    );
    let origin = state
        .gateway_origin(&caller.id)
        .expect("a live reach caller is a gateway origin");
    assert_eq!(origin.harness, caller.harness);
    let list = serde_json::from_value(serde_json::json!({"ListSessions": null})).unwrap();
    state
        .authorize_gateway_request(&caller.id, &list)
        .expect("room-level requests are admitted");
    let alias = serde_json::from_value(
        serde_json::json!({"AliasSession": {"session_id": "s", "alias": "a"}}),
    )
    .unwrap();
    state
        .authorize_gateway_request(&caller.id, &alias)
        .expect_err("sudo-class requests are not admitted for a bare reach caller");
    assert!(state.gateway_origin("reach:no-such-run").is_none());
    drop(reach.app);
}

#[tokio::test]
async fn renting_a_machine_asks_only_when_the_owner_turned_the_setting_on() {
    let reach = fixture();
    let acquire = serde_json::json!({"ControlPlane": {
        "action": "acquire", "capability_revision": "r1", "request_id": "one"}});
    // Off by default: the call reaches the control plane (a bare test kernel has none).
    let (ok, payload) = call(&reach, &reach.owner_token, None, acquire.clone()).await;
    assert!(!ok);
    assert_eq!(code(&payload), "request_failed", "{payload}");
    assert!(payload.to_string().contains("no connected machine service"));
    assert!(active_questions(&reach).is_empty());

    reach
        .router
        .runtime_state()
        .set_user_config_value("agents.ask_before_renting_machines".into(), "true".into())
        .await
        .expect("the owner turns the setting on");
    let asked = {
        let router = reach.router.clone();
        let token = reach.owner_token.clone();
        tokio::spawn(async move {
            router
                .dispatch_reach_tool_call(
                    &token,
                    "request",
                    serde_json::json!({"request": {"ControlPlane": {
                    "action": "acquire", "capability_revision": "r1", "request_id": "two"}}}),
                )
                .await
        })
    };
    let interaction = loop {
        if let Some(interaction) = active_questions(&reach).into_iter().next() {
            break interaction;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert!(interaction.message().starts_with("Rent a machine for"));
    let ids = interaction
        .choices()
        .iter()
        .map(|choice| choice.id().to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["accept", "decline"]);
    reach
        .router
        .runtime_state()
        .timeout_runtime_interaction(&reach.session_id, interaction.id())
        .await
        .expect("the owner's decision closes the card");
    let result = asked.await.unwrap().unwrap();
    assert!(!result.ok);
    assert_eq!(code(&result.payload), "rental_declined");
}

fn active_questions(reach: &Reach) -> Vec<crate::session::RuntimeInteraction> {
    reach
        .router
        .runtime_state()
        .session_snapshot_projection(&reach.session_id, 0)
        .expect("session projection should remain available")
        .session
        .active_interactions()
        .to_vec()
}
