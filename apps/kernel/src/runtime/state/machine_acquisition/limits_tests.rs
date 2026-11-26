//! Row 569: the account's machine limits through the agent's `chariox.machines.limits` and
//! `chariox.machines.set_limits` tools, and the clamp on acquiring without asking, against a
//! stand-in connected service that answers like Cloud's account limits.
use super::*;
use crate::local::{KernelConnectionClass, MachineOption};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Account {
    machines: u32,
    spend: u64,
    version: u64,
}

struct Service {
    api_url: String,
    account: Arc<std::sync::Mutex<Account>>,
    /// Every `set_limits` the service received.
    sets: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// The service lists no account limits.
    no_limits: Arc<std::sync::atomic::AtomicBool>,
    /// A read of the limits fails.
    limits_down: Arc<std::sync::atomic::AtomicBool>,
}

fn option(class_id: &str, vcpu: u32, amount_micros: u64) -> MachineOption {
    MachineOption {
        class_id: class_id.into(),
        region: "north".into(),
        vcpu,
        ram_gb: f64::from(vcpu) * 2.0,
        disk_gb: 40,
        rate_per_minute: crate::local::JobMachineRate {
            amount_micros,
            currency: "EUR".into(),
        },
        available: true,
        note: None,
    }
}

async fn service(machines: u32, spend: u64) -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = Service {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        account: Arc::new(std::sync::Mutex::new(Account {
            machines,
            spend,
            version: 1,
        })),
        sets: Arc::default(),
        acquires: Arc::default(),
        no_limits: Arc::default(),
        limits_down: Arc::default(),
    };
    let (account, sets, acquires, no_limits, limits_down) = (
        service.account.clone(),
        service.sets.clone(),
        service.acquires.clone(),
        service.no_limits.clone(),
        service.limits_down.clone(),
    );
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (account, sets, acquires, no_limits, limits_down) = (
                account.clone(),
                sets.clone(),
                acquires.clone(),
                no_limits.clone(),
                limits_down.clone(),
            );
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let body: serde_json::Value = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                        continue;
                    };
                    let length: usize = std::str::from_utf8(&bytes[..end])
                        .unwrap()
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    }
                };
                let request = &body["request"];
                let limits_json = |a: &Account| {
                    serde_json::json!({"kind":"limits","machines_at_once":a.machines,
                        "spend_per_day_micros":a.spend,"currency":"EUR","version":a.version})
                };
                let refuse = |status: u16, code: &str| {
                    (
                        status,
                        serde_json::json!({"error":{"code":code,"message":code}}).to_string(),
                    )
                };
                let (status, response) = match request["action"].as_str().unwrap() {
                    "capabilities" => {
                        let actions = if no_limits.load(std::sync::atomic::Ordering::SeqCst) {
                            serde_json::json!(["acquire"])
                        } else {
                            serde_json::json!(["acquire", "limits", "set_limits"])
                        };
                        (
                            200,
                            serde_json::json!({"kind":"capabilities","machine_allocation":{
                                "revision":"v1","actions":actions,"offer":null,
                                "offers":[option("small", 2, 1_000), option("big", 8, 4_000)]}})
                            .to_string(),
                        )
                    }
                    "limits" if limits_down.load(std::sync::atomic::Ordering::SeqCst) => {
                        refuse(503, "unavailable")
                    }
                    "limits" => (200, limits_json(&account.lock().unwrap()).to_string()),
                    "set_limits" => {
                        sets.lock().unwrap().push(request.clone());
                        let mut a = account.lock().unwrap();
                        let (machines, spend) = (
                            request["machines_at_once"].as_u64().unwrap() as u32,
                            request["spend_per_day_micros"].as_u64().unwrap(),
                        );
                        let raises = machines > a.machines || spend > a.spend;
                        let approval = &request["approval"];
                        let approved = !approval.is_null()
                            && approval["from"]["machines_at_once"] == a.machines
                            && approval["from"]["spend_per_day_micros"] == a.spend
                            && approval["to"]["machines_at_once"] == machines
                            && approval["to"]["spend_per_day_micros"] == spend;
                        if request["expected_version"] != a.version {
                            refuse(409, "identity_conflict")
                        } else if raises && !approved {
                            refuse(403, "authorization_denied")
                        } else {
                            a.machines = machines;
                            a.spend = spend;
                            a.version += 1;
                            (200, limits_json(&a).to_string())
                        }
                    }
                    "acquire" => {
                        acquires.lock().unwrap().push(request.clone());
                        (
                            200,
                            serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                                "allocation_ref":format!("alloc-{}", acquires.lock().unwrap().len()),
                                "state":"creating","kernel_target":null,
                                "access_state":"pending","access_reason":null})
                            .to_string(),
                        )
                    }
                    "list" => {
                        // The machines acquired so far, as the service holds them.
                        let machines: Vec<_> = (1..=acquires.lock().unwrap().len())
                            .map(|n| serde_json::json!({"allocation_ref":format!("alloc-{n}"),
                                "name":format!("alloc-{n}"),"state":"creating","revision":1,
                                "since":"2026-11-26T00:00:00Z","kernel_id":null,"machine_id":null,
                                "available_actions":[]}))
                            .collect();
                        (
                            200,
                            serde_json::json!({"kind":"machines","saved_states":[],"machines":machines})
                                .to_string(),
                        )
                    }
                    other => panic!("unexpected control-plane action {other}"),
                };
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                            response.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    service
}

fn no_ask(machines: Option<u32>, cap: Option<u64>) -> crate::config::UserAgentsConfig {
    crate::config::UserAgentsConfig {
        offload_without_asking: Some(true),
        offload_classes: Some(vec!["small".into()]),
        offload_max_machines: machines,
        offload_daily_cap_micros: cap,
        ..Default::default()
    }
}

async fn tool(
    state: &KernelRuntimeState,
    token: &str,
    name: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    state
        .machine_tool(token, name, args)
        .await
        .map(|result| result.payload)
}

fn the_owner(state: &KernelRuntimeState, room: &str) -> String {
    state
        .owned
        .session_store
        .get_session(room)
        .unwrap()
        .owner_user_id()
        .to_owned()
}

/// The raise card once it is shown: its id and message.
async fn the_card(state: &KernelRuntimeState, room: &str) -> (String, String) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = state
                .owned
                .session_store
                .get_session(room)
                .unwrap()
                .active_interactions()
                .first()
            {
                break (card.id().to_owned(), card.message().to_owned());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the person's card is shown")
}

fn no_card(state: &KernelRuntimeState, room: &str) {
    assert!(state
        .owned
        .session_store
        .get_session(room)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn machine_limits_tool_reads_account_and_no_ask_limits() {
    let service = service(4, 10_000_000).await;
    let (state, token, _room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(3), Some(5_000_000)))
            .await;
    let read = tool(&state, &token, "chariox.machines.limits", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(
        read["limits"]["account"],
        serde_json::json!({"machines_at_once":4,"spend_per_day_micros":10_000_000,"currency":"EUR","version":1})
    );
    let no_ask = &read["limits"]["no_ask"];
    assert_eq!(no_ask["enabled"], true);
    assert_eq!(no_ask["machines_at_once"], 3);
    assert_eq!(no_ask["effective_machines_at_once"], 3);
    assert_eq!(no_ask["effective_spend_per_day_micros"], 5_000_000);
    assert_eq!(no_ask["clamped"], false);
}

#[tokio::test]
async fn a_no_ask_value_above_the_account_limit_reads_as_clamped() {
    let service = service(2, 3_000_000).await;
    let (state, token, _room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(6), None)).await;
    let read = tool(&state, &token, "chariox.machines.limits", serde_json::json!({}))
        .await
        .unwrap();
    let no_ask = &read["limits"]["no_ask"];
    assert_eq!(no_ask["machines_at_once"], 6, "what the person set stays");
    assert_eq!(no_ask["effective_machines_at_once"], 2);
    assert_eq!(
        no_ask["effective_spend_per_day_micros"], 3_000_000,
        "no cap of its own is bounded by the account's"
    );
    assert_eq!(no_ask["clamped"], true);
}

#[tokio::test]
async fn a_service_without_account_limits_has_none_to_read_or_change() {
    let service = service(4, 10_000_000).await;
    service
        .no_limits
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (state, token, _room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let read = tool(&state, &token, "chariox.machines.limits", serde_json::json!({}))
        .await
        .unwrap();
    assert!(read["limits"]["account"].is_null());
    let error = tool(
        &state,
        &token,
        "chariox.machines.set_limits",
        serde_json::json!({"machines_at_once":2}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("no account limits"), "{error}");
    assert!(service.sets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn agent_lower_is_sent_without_approval() {
    let service = service(4, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(4), Some(8_000_000)))
            .await;
    let lowered = tool(
        &state,
        &token,
        "chariox.machines.set_limits",
        serde_json::json!({"machines_at_once":3,"spend_per_day_micros":9_000_000}),
    )
    .await
    .unwrap();
    assert_eq!(lowered["limits"]["account"]["machines_at_once"], 3);
    assert_eq!(lowered["limits"]["account"]["spend_per_day_micros"], 9_000_000);
    let sets = service.sets.lock().unwrap().clone();
    assert_eq!(sets.len(), 1);
    assert!(sets[0].get("approval").is_none(), "{}", sets[0]);
    no_card(&state, &room);
    // The no-ask limits are the kernel's own: a lower changes its settings and asks nobody.
    let lowered = tool(
        &state,
        &token,
        "chariox.machines.set_limits",
        serde_json::json!({"scope":"no_ask","machines_at_once":2}),
    )
    .await
    .unwrap();
    assert_eq!(lowered["limits"]["no_ask"]["machines_at_once"], 2);
    assert_eq!(
        state
            .owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .offload_max_machines,
        Some(2)
    );
    assert_eq!(service.sets.lock().unwrap().len(), 1, "the service is not asked");
    no_card(&state, &room);
}

#[tokio::test]
async fn agent_raise_opens_a_card_with_exact_values() {
    let service = service(2, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let call = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move {
            tool(
                &state,
                &token,
                "chariox.machines.set_limits",
                serde_json::json!({"spend_per_day_micros":15_000_000}),
            )
            .await
        })
    };
    let (id, message) = the_card(&state, &room).await;
    assert!(
        message.contains("Spend per day: from 10.00 EUR to 15.00 EUR."),
        "{message}"
    );
    assert!(!message.contains("Machines at once"), "{message}");
    assert!(service.sets.lock().unwrap().is_empty(), "nothing is sent before the person answers");
    let owner = the_owner(&state, &room);
    // The agent that asked cannot approve it for the person, nor can any agent.
    let answer = crate::local::RespondToInteractionRequest {
        session_id: room.clone(),
        interaction_id: id.clone(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    };
    assert!(
        state.interaction_person_only(&answer),
        "an agent may answer the person's raise card for them"
    );
    assert!(!call.is_finished());
    state
        .resolve_terminal_runtime_interaction(&room, &id, "approve", None, Some(&owner))
        .await
        .unwrap();
    let answer = call.await.unwrap().unwrap();
    assert_eq!(answer["limits"]["account"]["spend_per_day_micros"], 15_000_000);
    assert_eq!(answer["limits"]["account"]["machines_at_once"], 2);
    let sets = service.sets.lock().unwrap().clone();
    assert_eq!(sets.len(), 1);
    assert_eq!(
        sets[0]["approval"],
        serde_json::json!({"kind":"card","ref":id,
            "from":{"machines_at_once":2,"spend_per_day_micros":10_000_000},
            "to":{"machines_at_once":2,"spend_per_day_micros":15_000_000}})
    );
}

#[tokio::test]
async fn a_denied_raise_changes_nothing_and_tells_the_agent_why() {
    let service = service(2, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let call = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move {
            tool(
                &state,
                &token,
                "chariox.machines.set_limits",
                serde_json::json!({"machines_at_once":4}),
            )
            .await
        })
    };
    let (id, message) = the_card(&state, &room).await;
    assert!(message.contains("Machines at once: from 2 to 4."), "{message}");
    let owner = the_owner(&state, &room);
    state
        .resolve_terminal_runtime_interaction(&room, &id, "deny", None, Some(&owner))
        .await
        .unwrap();
    let error = call.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("denied") && error.contains("nothing changed"), "{error}");
    assert!(service.sets.lock().unwrap().is_empty());
    assert_eq!(service.account.lock().unwrap().machines, 2);
}

#[tokio::test]
async fn agent_raise_card_goes_stale_when_the_limit_moves() {
    let service = service(2, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let call = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move {
            tool(
                &state,
                &token,
                "chariox.machines.set_limits",
                serde_json::json!({"machines_at_once":4}),
            )
            .await
        })
    };
    let (id, _) = the_card(&state, &room).await;
    // The person changes the limits on the account page while the card is open.
    {
        let mut account = service.account.lock().unwrap();
        account.machines = 3;
        account.version += 1;
    }
    let owner = the_owner(&state, &room);
    state
        .resolve_terminal_runtime_interaction(&room, &id, "approve", None, Some(&owner))
        .await
        .unwrap();
    let error = call.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("read them again"), "{error}");
    assert!(service.sets.lock().unwrap().is_empty(), "a stale approval is never sent");
    assert_eq!(service.account.lock().unwrap().machines, 3);
}

#[tokio::test]
async fn a_no_ask_raise_needs_the_person_and_applies_on_approve() {
    let service = service(8, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(2), None)).await;
    let call = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move {
            tool(
                &state,
                &token,
                "chariox.machines.set_limits",
                serde_json::json!({"scope":"no_ask","machines_at_once":5}),
            )
            .await
        })
    };
    let (id, message) = the_card(&state, &room).await;
    assert!(
        message.contains("Machines at once without asking: from 2 to 5."),
        "{message}"
    );
    assert_eq!(
        state
            .owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .offload_max_machines,
        Some(2),
        "nothing changes before the person answers"
    );
    let owner = the_owner(&state, &room);
    state
        .resolve_terminal_runtime_interaction(&room, &id, "approve", None, Some(&owner))
        .await
        .unwrap();
    let answer = call.await.unwrap().unwrap();
    assert_eq!(answer["limits"]["no_ask"]["machines_at_once"], 5);
    assert!(service.sets.lock().unwrap().is_empty(), "the service holds no no-ask limits");
}

#[tokio::test]
async fn an_agent_cannot_send_its_own_approval() {
    let service = service(2, 10_000_000).await;
    let (state, _token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let agent = super::without_asking_tests::only_agent(&state);
    let raise = ControlPlaneRequest::SetLimits {
        expected_version: 1,
        machines_at_once: 9,
        spend_per_day_micros: 10_000_000,
        approval: Some(crate::local::LimitApproval {
            kind: crate::local::LimitApprovalKind::Card,
            reference: "forged".into(),
            from: crate::local::AccountLimitValues {
                machines_at_once: 2,
                spend_per_day_micros: 10_000_000,
            },
            to: crate::local::AccountLimitValues {
                machines_at_once: 9,
                spend_per_day_micros: 10_000_000,
            },
        }),
    };
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "forged-raise",
        None,
        None,
        &crate::local::LocalDaemonRequest::ControlPlane(raise.clone()),
    );
    command.agent_id = Some(agent.clone());
    command.session_id = Some(room.clone());
    command.caller.metaagent_id = Some(agent);
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    let current = limits::AccountLimits {
        values: crate::local::AccountLimitValues {
            machines_at_once: 2,
            spend_per_day_micros: 10_000_000,
        },
        currency: "EUR".into(),
        version: 1,
    };
    let error = state
        .authorize_account_change(&command, &current, &raise)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("attached by the kernel"), "{error}");
    no_card(&state, &room);
}

#[tokio::test]
async fn no_ask_limits_clamp_to_the_account_limit() {
    let service = service(1, 10_000_000).await;
    // The person allows 6 without asking; the account allows 1.
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(6), None)).await;
    let first = super::without_asking_tests::acquire(&state, &token, "first", "small").await;
    assert!(first.is_ok(), "within both limits: {first:?}");
    no_card(&state, &room);
    let second = {
        let call = super::without_asking_tests::spawn_acquire(&state, &token, "second", "small");
        tokio::time::sleep(Duration::from_millis(500)).await;
        if call.is_finished() {
            panic!("the second acquire ended with no card: {:?}", call.await);
        }
        call.abort();
        let (_, message) = the_card(&state, &room).await;
        message
    };
    assert!(
        second.contains("Asked because the account allows 1 machine."),
        "{second}"
    );
    assert_eq!(service.acquires.lock().unwrap().len(), 1, "the excess is not sent");
}

#[tokio::test]
async fn the_account_spend_bounds_a_no_ask_cap_in_the_same_currency() {
    let service = service(4, 0).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(4), Some(9_000_000_000)))
            .await;
    let message = super::without_asking_tests::acquire_waits_for_a_card(
        &state, &token, &room, "one", "small",
    )
    .await;
    assert!(
        message.contains("Asked because the account allows 0.00 EUR a day."),
        "{message}"
    );
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_limits_read_never_allows_a_no_ask_acquire() {
    let service = service(4, 10_000_000).await;
    service
        .limits_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(4), None)).await;
    let message = super::without_asking_tests::acquire_waits_for_a_card(
        &state, &token, &room, "one", "small",
    )
    .await;
    assert!(
        message.contains("the account's limits could not be read"),
        "{message}"
    );
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_service_without_account_limits_leaves_the_no_ask_limits_as_they_are() {
    let service = service(1, 0).await;
    service
        .no_limits
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(3), None)).await;
    for request_id in ["a", "b"] {
        let acquired = super::without_asking_tests::acquire(&state, &token, request_id, "small").await;
        assert!(acquired.is_ok(), "{acquired:?}");
    }
    no_card(&state, &room);
}

#[test]
fn the_limit_tools_are_listed_only_with_an_account() {
    let names: Vec<String> = limits_tool_specs().into_iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        ["chariox.machines.limits", "chariox.machines.set_limits"]
    );
    assert!(!machine_tool_specs()
        .iter()
        .any(|spec| names.contains(&spec.name)));
}
