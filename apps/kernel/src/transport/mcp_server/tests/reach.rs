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

const REACH_TOOL_NAMES: [&str; 7] = [
    "kernels",
    "sessions",
    "request",
    "events_subscribe",
    "events_wait",
    "events_ack",
    "guide",
];

const OWNER_TOKEN: &str = "reach-owner-token";
const GUEST_TOKEN: &str = "reach-guest-token";
const DRIVEN_TOKEN: &str = "reach-driven-token";

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
    fixture_with(true)
}

/// `owner_running` false leaves the owner's run parked, as a provider's run is while it starts.
fn fixture_with(owner_running: bool) -> Reach {
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
    let driven = {
        let mut sessions = app.sessions_mut();
        let mut agents = app.agents_mut();
        agents
            .create_agent(
                CreateAgentRequest::new(other_session.id(), "dev-stub").with_alias("driven-agent"),
                &mut sessions,
            )
            .expect("driven agent should be created")
    };
    let owner_token = launch(
        &mut app,
        session.id(),
        agent.id(),
        "reach-run-owner",
        OWNER_TOKEN,
    );
    launch(
        &mut app,
        other_session.id(),
        driven.id(),
        "reach-run-driven",
        DRIVEN_TOKEN,
    );
    if !owner_running {
        let mut providers = app.providers_mut();
        let mut run = providers
            .get_run("reach-run-owner")
            .expect("the owner's run exists");
        run.mark_parked();
        providers.insert_run_for_test(run);
    }
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
        reach_need(&parse(serde_json::json!(
            {"ReadKernelFile": {"path": "/etc/hostname", "offset": 0, "length": 1}}
        ))),
        ReachNeed::Sudo,
        "kernel-wide file access needs /sudo in the agent's own room"
    );
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"ControlPlane": {"action": "acquire",
            "capability_revision": "r", "request_id": "q"}})
        )),
        ReachNeed::Room,
        "acquiring a machine is room-level"
    );
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"ControlPlane": {"action": "delete_saved_state",
            "allocation_ref": "a", "saved_state_ref": "s", "capability_revision": "r",
            "request_id": "q", "expected_revision": 1}})
        )),
        ReachNeed::Sudo,
        "deleting a saved machine state needs /sudo"
    );
    assert_eq!(
        reach_need(&parse(serde_json::json!(
            {"RoomTerminal": {"session_id": "s", "operation": {"operation": "exec_list"}}}
        ))),
        ReachNeed::Room,
        "a room's own terminals are room-level"
    );
    assert_eq!(
        reach_need(&parse(
            serde_json::json!({"DeleteSession": {"session_ref": "s"}})
        )),
        ReachNeed::Excluded
    );
    for request in [
        serde_json::json!({"SubscribeEvents": {"targets": []}}),
        serde_json::json!({"WaitEvents": {"timeout_ms": 1}}),
        serde_json::json!({"AckEvents": {"cursor": 1}}),
        serde_json::json!({"RespondToInteraction": {
            "session_id": "s", "interaction_id": "i", "choice_id": "accept"}}),
    ] {
        assert_eq!(
            reach_need(&parse(request)),
            ReachNeed::Room,
            "following agents and answering a routine decision are room-level"
        );
    }
    assert_eq!(
        reach_need(&parse(serde_json::json!({"RespondToInteraction": {
            "session_id": "s", "interaction_id": "i", "choice_id": "approve",
            "passkey": "x"}}))),
        ReachNeed::HumanOnly,
        "a passkey stays with the person"
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
    assert_eq!(names, REACH_TOOL_NAMES);
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

/// One JSON-RPC call to `/mcp/reach` over a real connection to the runtime MCP server.
async fn http_rpc(port: u16, token: &str, body: Value) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the runtime MCP server accepts connections");
    stream
        .write_all(
            format!(
                "POST /mcp/reach HTTP/1.1\r\nHost: kernel\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("request is sent");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .await
        .expect("response is read");
    let (head, payload) = raw.split_once("\r\n\r\n").expect("an HTTP response");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    serde_json::from_str(payload.trim()).expect("a JSON-RPC body")
}

#[tokio::test]
async fn the_reach_server_serves_the_owners_agent_and_nobody_else_over_http() {
    let reach = fixture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(
        reach.router.clone(),
        listener,
    ));
    let list = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let owner = http_rpc(port, &reach.owner_token, list.clone()).await;
    let names = owner["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(names, REACH_TOOL_NAMES, "{owner}");
    let guest = http_rpc(port, &reach.guest_token, list).await;
    assert_eq!(guest["result"]["tools"], serde_json::json!([]), "{guest}");

    let call = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "request", "arguments": {"request": {"ListSessions": null}}}});
    let listed = http_rpc(port, &reach.owner_token, call.clone()).await;
    assert_eq!(listed["result"]["isError"], false, "{listed}");
    assert!(listed.to_string().contains(&reach.other_session_id));
    let refused = http_rpc(port, &reach.guest_token, call).await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(refused.to_string().contains("reach_not_available"));
    server.abort();
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
async fn acquiring_a_machine_asks_unless_the_owner_turned_the_setting_off() {
    let reach = fixture();
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
    assert!(interaction.message().starts_with("Let "));
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
    assert_eq!(code(&result.payload), "acquisition_declined");

    reach
        .router
        .runtime_state()
        .set_user_config_value("agents.ask_before_acquiring_machines".into(), "false".into())
        .await
        .expect("the owner turns the setting off");
    let acquire = serde_json::json!({"ControlPlane": {
        "action": "acquire", "capability_revision": "r1", "request_id": "one"}});
    // Off: the call reaches the control plane (a bare test kernel has none) with no question.
    let (ok, payload) = call(&reach, &reach.owner_token, None, acquire).await;
    assert!(!ok);
    assert_eq!(code(&payload), "request_failed", "{payload}");
    assert!(payload.to_string().contains("no connected machine service"));
    assert!(active_questions(&reach).is_empty());
}

#[test]
fn the_service_names_a_machine_after_the_agent_that_acquired_it() {
    let reach = fixture();
    let state = reach.router.runtime_state();
    let caller = state
        .reach_for_auth_token(&reach.owner_token)
        .expect("the owner's run is a reach caller");
    let acquire = crate::local::ControlPlaneRequest::Acquire {
        capability_revision: "r1".into(),
        request_id: "one".into(),
    };
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "c",
        None,
        Some(caller.id.clone()),
        &crate::local::LocalDaemonRequest::ControlPlane(acquire.clone()),
    );
    command.caller = crate::runtime::command::KernelCaller::for_source(
        &crate::runtime::command::KernelCommandSource::LocalIpc,
    )
    .with_connection_class(crate::local::KernelConnectionClass::KernelAgent);
    command.caller.caller_id = caller.id.clone();
    let sent = crate::runtime::control_plane::service_request(&state, &command, &acquire);
    assert_eq!(sent["requested_by"], caller.harness);
    assert!(caller.harness.ends_with(" (Chariox agent)"), "{}", caller.harness);
    let list = crate::runtime::control_plane::service_request(&state, &command, &crate::local::ControlPlaneRequest::List {});
    assert!(list.get("requested_by").is_none(), "only an acquisition carries a name");
    let own = crate::runtime::command::KernelCommand::from_local_request(
        "d",
        None,
        None,
        &crate::local::LocalDaemonRequest::ControlPlane(acquire.clone()),
    );
    let sent = crate::runtime::control_plane::service_request(&state, &own, &acquire);
    assert!(sent.get("requested_by").is_none(), "the owner's own client sends no name");
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

async fn tool(reach: &Reach, name: &str, arguments: Value) -> (bool, Value) {
    let result = reach
        .router
        .dispatch_reach_tool_call(&reach.owner_token, name, arguments)
        .await
        .expect("reach call should return a tool result");
    (result.ok, result.payload)
}

#[tokio::test]
async fn the_sessions_tool_lists_rooms_and_one_rooms_agents() {
    let reach = fixture();
    let (ok, rooms) = tool(&reach, "sessions", serde_json::json!({})).await;
    assert!(ok, "{rooms}");
    assert!(
        rooms.to_string().contains(&reach.other_session_id),
        "{rooms}"
    );
    let (ok, agents) = tool(
        &reach,
        "sessions",
        serde_json::json!({"session_id": reach.session_id}),
    )
    .await;
    assert!(ok, "{agents}");
    assert!(agents["AgentsListed"]["agents"].is_array(), "{agents}");
}

#[tokio::test]
async fn the_guide_tool_answers_with_every_tool_named() {
    let reach = fixture();
    let (ok, payload) = tool(&reach, "guide", serde_json::json!({})).await;
    assert!(ok);
    let guide = payload["guide"].as_str().expect("a guide text");
    for name in REACH_TOOL_NAMES {
        assert!(
            guide.contains(&format!("{name}(")),
            "{name} is missing from the guide"
        );
    }
}

/// The tools come from the CLI command table: every property of a schema is one the
/// kernel reads, and every example request the guide shows is one an agent may send.
#[tokio::test]
async fn the_generated_tools_match_what_the_kernel_reads() {
    use crate::runtime::state::reach::{reach_need, ReachNeed};
    let reach = fixture();
    let sample = |schema: &Value| match schema["type"].as_str() {
        Some("integer") => serde_json::json!(1),
        Some("object") => serde_json::json!({"ListSessions": null}),
        _ => serde_json::json!("x"),
    };
    for spec in crate::transport::runtime_tools::reach_tool_specs() {
        assert_eq!(
            spec.input_schema["additionalProperties"], false,
            "{}",
            spec.name
        );
        let properties = spec.input_schema["properties"]
            .as_object()
            .expect("properties");
        let arguments = Value::Object(
            properties
                .iter()
                .map(|(name, schema)| (name.clone(), sample(schema)))
                .collect(),
        );
        let result = reach
            .router
            .dispatch_reach_tool_call(&reach.owner_token, &spec.name, arguments)
            .await;
        // A refusal or a kernel answer is fine; an argument the kernel does not read is not.
        if let Err(error) = result {
            panic!("{} does not read its own schema: {error}", spec.name);
        }
    }
    let generated: Value =
        serde_json::from_str(include_str!("../../runtime_tools/reach_tools.json"))
            .expect("the generated tools are JSON");
    for example in generated["examples"].as_array().expect("examples") {
        let example = &example["request"];
        let request: crate::local::LocalDaemonRequest =
            serde_json::from_value(example.clone()).expect("an example is a kernel request");
        assert!(
            matches!(reach_need(&request), ReachNeed::Room | ReachNeed::Sudo),
            "an agent may not send the example {example}"
        );
    }
}

#[test]
fn the_events_tools_ask_the_inbox_of_the_agents_own_kernel() {
    use crate::transport::runtime_tools::*;
    assert_eq!(
        reach_events_wait_request(ReachEventsWaitArgs {
            since: Some(4),
            limit: None,
            timeout_seconds: None
        }),
        serde_json::json!({"WaitEvents": {"since": 4, "limit": 1000, "timeout_ms": 60000}})
    );
    assert_eq!(
        reach_events_wait_request(ReachEventsWaitArgs {
            since: None,
            limit: Some(5),
            timeout_seconds: Some(900)
        }),
        serde_json::json!({"WaitEvents": {"limit": 5, "timeout_ms": 280000}}),
        "a wait never lasts longer than a provider lets a tool call last"
    );
    assert_eq!(
        reach_events_subscribe_request(ReachEventsSubscribeArgs {
            kernel: "k".into(),
            session_id: "s".into(),
            agent_id: Some("a".into()),
        }),
        serde_json::json!({"SubscribeEvents": {"targets": [{"kernel": "k", "session_id": "s", "agent_id": "a"}]}})
    );
    assert_eq!(
        reach_events_ack_request(ReachEventsAckArgs { cursor: 9 }),
        serde_json::json!({"AckEvents": {"cursor": 9}})
    );
    for request in [
        reach_events_wait_request(ReachEventsWaitArgs {
            since: None,
            limit: None,
            timeout_seconds: None,
        }),
        reach_events_ack_request(ReachEventsAckArgs { cursor: 1 }),
    ] {
        serde_json::from_value::<crate::local::LocalDaemonRequest>(request)
            .expect("an events tool sends a kernel request");
    }
}

#[tokio::test]
async fn a_room_tool_given_another_kernel_is_refused() {
    let reach = fixture();
    for tool_name in ["chariox_list_session_agents", "chariox_kernel_request"] {
        let error = reach
            .router
            .dispatch_authenticated_runtime_tool_call(
                &reach.owner_token,
                tool_name,
                serde_json::json!({"kernel": "kernel-b", "request": {"ListSessions": null}}),
            )
            .await
            .expect_err("a room tool does not take a kernel");
        assert!(
            error.to_string().contains("chariox-reach"),
            "{tool_name}: {error}"
        );
    }
    // Without a kernel the same call is the room's own.
    let listed = reach
        .router
        .dispatch_authenticated_runtime_tool_call(
            &reach.owner_token,
            "chariox_list_session_agents",
            serde_json::json!({}),
        )
        .await
        .expect("the room tool still works");
    assert!(listed.ok);
}

/// OpenCode lists its tools while its run is still starting and keeps the answer.
#[tokio::test]
async fn the_reach_tools_are_listed_before_the_run_is_running_and_called_only_while_it_runs() {
    let reach = fixture_with(false);
    let names = reach
        .router
        .reach_tool_specs_for_auth_token(&reach.owner_token)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(names, REACH_TOOL_NAMES);
    let (ok, payload) = tool(&reach, "sessions", serde_json::json!({})).await;
    assert!(!ok);
    assert_eq!(code(&payload), "reach_not_available");
    assert!(
        reach
            .router
            .reach_tool_specs_for_auth_token(&reach.guest_token)
            .is_empty(),
        "a guest's agent gets none, whatever state its run is in"
    );
}

/// AT-40 on the reach surface: a driver follows the room of an agent it drives, is woken by
/// the decision that agent raises, reads it and answers it; nothing from other rooms arrives.
#[tokio::test]
async fn a_driver_is_woken_by_the_decision_a_driven_agent_raises_and_answers_it() {
    let reach = fixture();
    let (ok, subscription) = tool(
        &reach,
        "events_subscribe",
        serde_json::json!({"kernel": "local", "session_id": reach.other_session_id}),
    )
    .await;
    assert!(ok, "{subscription}");

    let driven = {
        let router = reach.router.clone();
        let token = DRIVEN_TOKEN;
        tokio::spawn(async move {
            router
                .dispatch_reach_tool_call(
                    token,
                    "request",
                    serde_json::json!({"request": {"ControlPlane": {
                    "action": "acquire", "capability_revision": "r1", "request_id": "driven"}}}),
                )
                .await
        })
    };
    let (ok, batch) = tool(
        &reach,
        "events_wait",
        serde_json::json!({"timeout_seconds": 20}),
    )
    .await;
    assert!(ok, "{batch}");
    let events = batch["EventsListed"]["events"].as_array().expect("events");
    assert!(
        events
            .iter()
            .all(|event| event["session_id"] == reach.other_session_id),
        "only the followed room reports: {batch}"
    );
    let card = events
        .iter()
        .find(|event| event["type"] == "interaction" && event["value"]["state"] == "pending")
        .unwrap_or_else(|| panic!("the pending decision wakes the driver: {batch}"));
    assert!(card["value"]["question"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Let "));
    assert!(card["agent_id"].is_string() && card["kernel"].is_string());
    let interaction_id = card["refs"]["interaction_id"]
        .as_str()
        .expect("interaction id");

    let (ok, answered) = tool(
        &reach,
        "request",
        serde_json::json!({"request": {"RespondToInteraction": {
            "session_id": reach.other_session_id,
            "interaction_id": interaction_id,
            "choice_id": "accept"}}}),
    )
    .await;
    assert!(ok, "{answered}");
    let result = driven.await.unwrap().unwrap();
    assert_ne!(
        code(&result.payload),
        "acquisition_declined",
        "the driver's answer reached the driven agent: {}",
        result.payload
    );
    let cursor = batch["EventsListed"]["next_cursor"]
        .as_u64()
        .expect("cursor");
    let (ok, _) = tool(&reach, "events_ack", serde_json::json!({"cursor": cursor})).await;
    assert!(ok);
}

#[tokio::test]
async fn an_agent_does_not_answer_the_decision_of_its_own_room() {
    let reach = fixture();
    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        None,
        serde_json::json!({"RespondToInteraction": {
            "session_id": reach.session_id, "interaction_id": "any", "choice_id": "accept"}}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "excluded_for_agents");
}

#[tokio::test]
async fn events_follow_the_run_and_run_on_the_agents_own_kernel() {
    let reach = fixture();
    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        Some("kernel-b"),
        serde_json::json!({"WaitEvents": {"timeout_ms": 1}}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "events_local_only");
    // A guest's room is not the owner's to follow.
    let (ok, payload) = tool(
        &reach,
        "events_subscribe",
        serde_json::json!({"kernel": "local", "session_id": "no-such-room"}),
    )
    .await;
    assert!(!ok);
    assert_eq!(code(&payload), "events_refused", "{payload}");

    let (ok, _) = tool(
        &reach,
        "events_subscribe",
        serde_json::json!({"kernel": "local", "session_id": reach.other_session_id}),
    )
    .await;
    assert!(ok);
    let state = reach.router.runtime_state();
    let id = state.reach_for_auth_token(&reach.owner_token).unwrap().id;
    // The run ends: its inbox ends with it, and nothing waits on a run that is gone.
    {
        let app = reach.app.lock().await;
        let mut providers = app.providers_mut();
        let mut run = providers.get_run("reach-run-owner").unwrap();
        run.mark_parked();
        providers.insert_run_for_test(run);
    }
    state.end_dead_reach_subscriptions().unwrap();
    let ended = state.outside_ended_response(&id).unwrap();
    assert!(ended.is_some(), "the subscription of an ended run ends");
}

#[tokio::test]
async fn a_driver_does_not_answer_a_decision_only_the_person_may_answer() {
    let reach = fixture();
    let popup = {
        let router = reach.router.clone();
        tokio::spawn(async move {
            super::permission_prompt::rpc(
                &router,
                DRIVEN_TOKEN,
                "tools/call",
                serde_json::json!({"name": "chariox.request_popup", "arguments": {
                    "message": "Wipe the machine?", "level": "critical", "timeout_sec": 30,
                    "choices": [{"id": "yes", "label": "Yes", "reply": "yes"}]}}),
            )
            .await
        })
    };
    let interaction = loop {
        let open = reach
            .router
            .runtime_state()
            .session_snapshot_projection(&reach.other_session_id, 0)
            .expect("session projection should remain available")
            .session
            .active_interactions()
            .to_vec();
        if let Some(interaction) = open.into_iter().next() {
            break interaction;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let (ok, payload) = call(
        &reach,
        &reach.owner_token,
        None,
        serde_json::json!({"RespondToInteraction": {
            "session_id": reach.other_session_id,
            "interaction_id": interaction.id(),
            "choice_id": "yes"}}),
    )
    .await;
    assert!(!ok, "{payload}");
    assert!(
        payload
            .to_string()
            .contains("agents cannot answer approvals"),
        "{payload}"
    );
    reach
        .router
        .runtime_state()
        .timeout_runtime_interaction(&reach.other_session_id, interaction.id())
        .await
        .expect("the person's side closes the card");
    popup.await.unwrap();
}
