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
    /// The service says how many machines are in use of the account's limit, with its offer and its limits.
    counts: Arc<std::sync::atomic::AtomicBool>,
    move_usage_on_third_read: Arc<std::sync::atomic::AtomicBool>,
    limits_reads: Arc<std::sync::atomic::AtomicUsize>,
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
        counts: Arc::default(),
        move_usage_on_third_read: Arc::default(),
        limits_reads: Arc::default(),
    };
    let counts_flag = service.counts.clone();
    let move_usage = service.move_usage_on_third_read.clone();
    let limit_reads = service.limits_reads.clone();
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
            let counts = counts_flag.clone();
            let move_usage = move_usage.clone();
            let limit_reads = limit_reads.clone();
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
                if request["action"] == "limits" {
                    let read = limit_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if read == 2 && move_usage.load(std::sync::atomic::Ordering::SeqCst) {
                        acquires.lock().unwrap().push(serde_json::json!({"request_id":"concurrent-usage"}));
                    }
                }
                let in_use = acquires.lock().unwrap().len();
                let counted = counts.load(std::sync::atomic::Ordering::SeqCst);
                let limits_json = |a: &Account| {
                    let mut limits = serde_json::json!({"kind":"limits","machines_at_once":a.machines,
                        "spend_per_day_micros":a.spend,"currency":"EUR","version":a.version});
                    if counted {
                        limits["machines_in_use"] = in_use.into();
                        limits["spend_today_micros"] = (in_use as u64 * 500_000).into();
                    }
                    limits
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
                        let mut capability = serde_json::json!({"kind":"capabilities","machine_allocation":{
                            "revision":"v1","actions":actions,"offer":null,
                            "offers":[option("small", 2, 1_000), option("big", 8, 4_000)]}});
                        if counted {
                            capability["machine_allocation"]["account_machines"] =
                                serde_json::json!({"in_use":in_use,"allowed":account.lock().unwrap().machines});
                            capability["machine_allocation"]["account_spend"] = serde_json::json!({
                                "used_micros": in_use as u64 * 500_000,
                                "allowed_micros": account.lock().unwrap().spend, "currency": "EUR"});
                        }
                        (200, capability.to_string())
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
                        // Like the service: a request_id it accepted answers with its own allocation.
                        let mut seen = acquires.lock().unwrap();
                        let known = seen.iter().position(|r| r["request_id"] == request["request_id"]);
                        let n = known.unwrap_or_else(|| {
                            seen.push(request.clone());
                            seen.len() - 1
                        }) + 1;
                        (
                            200,
                            serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                                "allocation_ref":format!("alloc-{n}"),
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

/// The existing synthetic running-window precondition, bound to the actual admitted prompt.
/// The public tool still derives its agent and verifies this window; this is not live approval proof.
fn named_window(state: &KernelRuntimeState, room: &str, text: &str) -> String {
    let agent = super::without_asking_tests::only_agent(state);
    let session = state.owned.session_store.get_session(room).unwrap();
    let prompt = if let Some(prompt) = state.owned.prompt_state_owner.active_prompt_for_agent(&session, &agent) {
        prompt
    } else {
        let attachment = state.attachment_store.list_session_attachment_ids(room)
            .into_iter().next().expect("the fixture has an attachment");
        let pending = crate::session::PromptQueueItem::new(
            "limits-owner-task", &attachment, &agent, "change the named limits", crate::session::PromptStatus::Queued,
        );
        match state.owned.prompt_state_owner.submit_prepared_prompt_with_queue_policy(&session, pending, false, false).unwrap() {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            other => panic!("a fresh fixture starts its exact owner task: {other:?}"),
        }
    };
    let entry = "sudo:limits-fixture".to_owned();
    let run = state.owned.provider_store.get_run_for_agent(room, &agent).unwrap();
    let turn = crate::local::KernelSudoTurn {
        entry_id: entry.clone(), session_id: room.into(), agent_id: agent.clone(),
        owner_user_id: session.owner_user_id().into(), terminal_id: "limits-owner-terminal".into(),
        requester: None, prompt_id: Some(prompt.id().into()), provider_run_id: Some(run.id().into()),
        task_id: None, duration_minutes: 60, expires_at_ms: Some(crate::session::unix_epoch_ms() + 3_600_000),
        revision: 1, warning_sent: false, deadline: Some(std::time::Instant::now() + Duration::from_secs(3600)),
        placement: None, limit_grant: Some(crate::runtime::state::LimitGrant::parse(text).expect("the exact fixture grant parses")),
    };
    assert!(state.owned.prompt_state_owner.bind_sudo_turn(&session, &agent, prompt.id(), &entry));
    state.owned.sudo_turns.lock().unwrap().insert(entry.clone(), turn);
    entry
}

fn owner_step(error: &DaemonError) {
    assert!(matches!(error, DaemonError::MachineRequest { code, .. } if code == "owner_action_required"), "{error}");
    assert!(error.to_string().contains("/sudo"), "the refusal supplies the exact owner step: {error}");
}

async fn change(state: &KernelRuntimeState, token: &str, args: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
    tokio::time::timeout(Duration::from_secs(5), tool(state, token, "chariox.machines.set_limits", args))
        .await.expect("a limits request settles without opening a new decision")
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
async fn agent_raise_needs_the_exact_named_owner_window() {
    let service = service(2, 10_000_000).await;
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let args = serde_json::json!({"spend_per_day_micros":15_000_000});
    let error = change(&state, &token, args.clone()).await.unwrap_err();
    owner_step(&error);
    assert!(error.to_string().contains("15") && error.to_string().contains("EUR"));
    assert!(service.sets.lock().unwrap().is_empty(), "nothing is sent without the named owner authority");
    no_card(&state, &room);
    let entry = named_window(&state, &room, "raise account limits to 15 EUR a day");
    let answer = change(&state, &token, args).await.unwrap();
    assert_eq!(answer["limits"]["account"]["spend_per_day_micros"], 15_000_000);
    assert_eq!(answer["limits"]["account"]["machines_at_once"], 2);
    let sets = service.sets.lock().unwrap().clone();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0]["approval"], serde_json::json!({"kind":"sudo","ref":entry,
        "from":{"machines_at_once":2,"spend_per_day_micros":10_000_000},
        "to":{"machines_at_once":2,"spend_per_day_micros":15_000_000}}));
    no_card(&state, &room);
}

#[tokio::test]
async fn an_unnamed_or_insufficient_raise_changes_nothing() {
    let service = service(2, 10_000_000).await;
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    for grant in [None, Some("raise account limits to 3 machines"), Some("raise no-ask limits to 4 machines")] {
        if let Some(text) = grant { named_window(&state, &room, text); }
        owner_step(&change(&state, &token, serde_json::json!({"machines_at_once":4})).await.unwrap_err());
        assert!(service.sets.lock().unwrap().is_empty());
        assert_eq!(service.account.lock().unwrap().machines, 2);
        no_card(&state, &room);
    }
}

#[tokio::test]
async fn an_agent_cannot_send_an_obsolete_expected_limit_version() {
    let service = service(2, 10_000_000).await;
    let (state, _token, room, _worktree) = super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    named_window(&state, &room, "raise account limits to 4 machines");
    { let mut account = service.account.lock().unwrap(); account.machines = 3; account.version += 1; }
    let request = ControlPlaneRequest::SetLimits { expected_version:1, machines_at_once:4, spend_per_day_micros:10_000_000, approval:None };
    let agent = super::without_asking_tests::only_agent(&state);
    let mut command = crate::runtime::command::KernelCommand::from_local_request("stale-limits", None, None, &crate::local::LocalDaemonRequest::ControlPlane(request.clone()));
    command.agent_id=Some(agent.clone()); command.session_id=Some(room.clone()); command.caller.metaagent_id=Some(agent); command.caller.connection_class=Some(KernelConnectionClass::KernelAgent);
    let current = limits::AccountLimits { values:crate::local::AccountLimitValues {machines_at_once:3,spend_per_day_micros:10_000_000}, currency:"EUR".into(),version:2,machines_in_use:None,spend_today_micros:None };
    let error = state.authorize_account_change(&command, &current, &request).await.unwrap_err().to_string();
    assert!(error.contains("read them again"), "{error}");
    assert!(service.sets.lock().unwrap().is_empty(), "a stale approval is never sent");
    assert_eq!(service.account.lock().unwrap().machines, 3);
    no_card(&state, &room);
}

#[tokio::test]
async fn a_no_ask_raise_needs_its_own_named_owner_scope() {
    let service = service(8, 10_000_000).await;
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(Some(2), None)).await;
    let args = serde_json::json!({"scope":"no_ask","machines_at_once":5});
    named_window(&state, &room, "raise account limits to 5 machines");
    owner_step(&change(&state, &token, args.clone()).await.unwrap_err());
    assert_eq!(no_ask_max(&state), Some(2), "account authority cannot raise no-ask limits");
    no_card(&state, &room);
    named_window(&state, &room, "raise no-ask limits to 5 machines");
    let answer = change(&state, &token, args).await.unwrap();
    assert_eq!(answer["limits"]["no_ask"]["machines_at_once"], 5);
    assert!(service.sets.lock().unwrap().is_empty(), "the service holds no no-ask limits");
    no_card(&state, &room);
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
        machines_in_use: None,
        spend_today_micros: None,
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

#[tokio::test]
async fn the_options_and_the_card_say_how_many_machines_are_in_use_before_anyone_is_asked() {
    let service = service(3, 10_000_000).await;
    service.counts.store(true, std::sync::atomic::Ordering::SeqCst);
    // Not allowed without asking: every acquire goes to the person's card.
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(
        service.api_url.clone(),
        crate::config::UserAgentsConfig::default(),
    )
    .await;
    let options = tool(&state, &token, "chariox.machines.options", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(options["account_machines"], serde_json::json!({"in_use":0,"allowed":3}));
    assert_eq!(
        options["account_spend"],
        serde_json::json!({"used_micros":0,"allowed_micros":10_000_000,"currency":"EUR"})
    );
    let read = tool(&state, &token, "chariox.machines.limits", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(read["limits"]["account"]["machines_in_use"], 0);
    assert_eq!(read["limits"]["account"]["spend_today_micros"], 0);
    let message = super::without_asking_tests::acquire_waits_for_a_card(
        &state, &token, &room, "one", "small",
    )
    .await;
    assert!(message.contains("The account has 0 of 3 machines in use."), "{message}");
    assert!(message.contains("0.00 of 10.00 EUR spent today."), "{message}");
}

#[tokio::test]
async fn a_full_account_refuses_an_acquire_before_the_person_is_asked_and_a_retry_still_replays() {
    let service = service(1, 10_000_000).await;
    service.counts.store(true, std::sync::atomic::Ordering::SeqCst);
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(
        service.api_url.clone(),
        no_ask(Some(4), None),
    )
    .await;
    // Within the person's no-ask limits and the account's: the one machine goes ahead.
    let first = super::without_asking_tests::acquire(&state, &token, "first", "small").await;
    assert!(first.is_ok(), "{first:?}");
    // Now 1 of 1 is in use: a second is refused before any card, naming the count and the way out.
    let error = super::without_asking_tests::acquire(&state, &token, "second", "small")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("allows 1 machine and 1 in use") && error.contains("not asked of the person"),
        "{error}"
    );
    no_card(&state, &room);
    assert_eq!(service.acquires.lock().unwrap().len(), 1, "the refused one is not sent");
    // The first request, retried after a lost answer, replays: the full account does not refuse it.
    let again = super::without_asking_tests::acquire(&state, &token, "first", "small").await;
    assert!(again.is_ok(), "{again:?}");
}

/// A change of a setting reloads the kernel's config from its app, which this fixture gave no Cloud sign-in: put it back.
fn keep_the_account(state: &KernelRuntimeState, account: &Option<crate::config::PersistedCloudRelayProfile>) {
    let mut config = state.owned.config_projection.snapshot();
    config.cloud_relay = account.clone();
    state.owned.config_projection.update(config);
}

fn classes_of(state: &KernelRuntimeState) -> Option<Vec<String>> {
    state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .agents
        .offload_classes
}

#[tokio::test]
async fn no_ask_classes_lower_directly_and_need_the_exact_named_target_to_expand() {
    let service = service(8, 10_000_000).await;
    let settings = crate::config::UserAgentsConfig {offload_classes:Some(vec!["small".into(),"big".into()]), ..no_ask(None,None)};
    let (state, token, room, _worktree) = super::without_asking_tests::agent_kernel(service.api_url.clone(), settings).await;
    let account = state.owned.config_projection.snapshot().cloud_relay;
    let lowered = change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small"]})).await.unwrap();
    assert_eq!(lowered["limits"]["no_ask"]["classes"], serde_json::json!(["small"]));
    assert_eq!(classes_of(&state),Some(vec!["small".into()])); keep_the_account(&state,&account); no_card(&state,&room);
    for text in ["raise no-ask limits to 8 machines", "raise account limits to 8 machines", "raise no-ask limits to classes [\"small\"]"] {
        named_window(&state,&room,text);
        owner_step(&change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small","big"]})).await.unwrap_err());
        assert_eq!(classes_of(&state),Some(vec!["small".into()])); no_card(&state,&room);
    }
    named_window(&state,&room,"raise no-ask limits to classes [\"big\",\"small\"]");
    let answer=change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small","big"]})).await.unwrap();
    assert_eq!(answer["limits"]["no_ask"]["classes"],serde_json::json!(["small","big"]));
    assert_eq!(classes_of(&state),Some(vec!["small".into(),"big".into()])); keep_the_account(&state,&account);
    let reset=change(&state,&token,serde_json::json!({"scope":"no_ask","classes":[]})).await.unwrap();
    assert!(reset["limits"]["no_ask"]["classes"].is_null()); assert_eq!(classes_of(&state),None); no_card(&state,&room);
}

#[tokio::test]
async fn a_class_the_service_does_not_offer_and_classes_of_the_account_scope_are_refused() {
    let service = service(8, 10_000_000).await;
    let (state, token, room, _worktree) =
        super::without_asking_tests::agent_kernel(service.api_url.clone(), no_ask(None, None)).await;
    let error = tool(
        &state,
        &token,
        "chariox.machines.set_limits",
        serde_json::json!({"scope":"no_ask","classes":["huge"]}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("huge is not a machine class the service offers"), "{error}");
    let error = tool(
        &state,
        &token,
        "chariox.machines.set_limits",
        serde_json::json!({"classes":["small"]}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("scope no_ask"), "{error}");
    assert_eq!(classes_of(&state), Some(vec!["small".into()]));
    no_card(&state, &room);
}



fn no_ask_max(state: &KernelRuntimeState) -> Option<u32> {
    state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .agents
        .offload_max_machines
}

#[tokio::test]
async fn a_named_approval_is_not_stale_when_only_usage_moves_between_reads() {
    let service=service(2,10_000_000).await;
    service.counts.store(true,std::sync::atomic::Ordering::SeqCst);
    service.move_usage_on_third_read.store(true,std::sync::atomic::Ordering::SeqCst);
    let (state,token,room,_worktree)=super::without_asking_tests::agent_kernel(service.api_url.clone(),crate::config::UserAgentsConfig::default()).await;
    named_window(&state,&room,"raise account limits to 15 EUR a day");
    let answer=change(&state,&token,serde_json::json!({"spend_per_day_micros":15_000_000})).await.unwrap();
    assert_eq!(answer["limits"]["account"]["spend_per_day_micros"],15_000_000);
    assert!(service.limits_reads.load(std::sync::atomic::Ordering::SeqCst)>=3, "the actual request rechecks the limits");
    assert_eq!(service.acquires.lock().unwrap().len(),1,"usage really changed between the HTTP reads");
    assert_eq!(service.sets.lock().unwrap().len(),1); no_card(&state,&room);
}

#[tokio::test]
async fn a_combined_no_ask_change_requires_one_grant_for_every_raise_before_any_write() {
    let service=service(8,10_000_000).await;
    let settings=crate::config::UserAgentsConfig {offload_classes:Some(vec!["small".into(),"big".into()]),..no_ask(Some(2),None)};
    let (state,token,room,_worktree)=super::without_asking_tests::agent_kernel(service.api_url.clone(),settings).await;
    let account=state.owned.config_projection.snapshot().cloud_relay;
    owner_step(&change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small"],"machines_at_once":5})).await.unwrap_err());
    assert_eq!(classes_of(&state),Some(vec!["small".into(),"big".into()]),"the lower did not apply alone"); assert_eq!(no_ask_max(&state),Some(2));
    change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small"]})).await.unwrap(); keep_the_account(&state,&account);
    let before=state.owned.config_projection.snapshot().user_config.agents;
    for text in ["raise no-ask limits to classes [\"small\",\"big\"]", "raise no-ask limits to 5 machines"] {
        named_window(&state,&room,text);
        owner_step(&change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small","big"],"machines_at_once":5})).await.unwrap_err());
        assert_eq!(state.owned.config_projection.snapshot().user_config.agents,before,"alternating partial grants never change either field"); no_card(&state,&room);
    }
    named_window(&state,&room,"raise no-ask limits to 5 machines and classes [\"small\",\"big\"]");
    change(&state,&token,serde_json::json!({"scope":"no_ask","classes":["small","big"],"machines_at_once":5})).await.unwrap();
    assert_eq!(classes_of(&state),Some(vec!["small".into(),"big".into()])); assert_eq!(no_ask_max(&state),Some(5)); no_card(&state,&room);
}

#[tokio::test]
async fn a_named_no_ask_raise_never_overwrites_a_newer_owner_setting() {
    use std::future::Future;
    let service=service(8,10_000_000).await;
    let (state,_token,room,_worktree)=super::without_asking_tests::agent_kernel(service.api_url.clone(),no_ask(Some(2),None)).await;
    named_window(&state,&room,"raise no-ask limits to 5 machines");
    let capability: MachineAllocationCapability=serde_json::from_value(serde_json::json!({
        "revision":"v1","actions":["acquire","limits","set_limits"],"offer":null,
        "offers":[option("small",2,1_000),option("big",8,4_000)]
    })).unwrap();
    let caller=AgentCaller {room:room.clone(),owner:the_owner(&state,&room),agent:Some(super::without_asking_tests::only_agent(&state))};
    let app=Arc::clone(&state.app); let mut held=app.lock().await;
    // Poll the real transaction until it waits for the existing app mutex after planning.
    let mut change=Box::pin(state.change_no_ask_limits(Some(caller),Some(5),None,None,&capability,None));
    let mut context=std::task::Context::from_waker(std::task::Waker::noop());
    assert!(change.as_mut().poll(&mut context).is_pending(),"the planned write waits on the app mutex");
    // The owner's normal app settings entry writes under its held mutex; no raw store or hook.
    held.set_user_config_value("agents.offload_max_machines","1".into()).unwrap();
    let owner_config=held.config().clone(); drop(held); state.owned.config_projection.update(owner_config);
    let error=change.await.unwrap_err().to_string();
    assert!(error.contains("changed while this was open"),"{error}");
    assert_eq!(no_ask_max(&state),Some(1),"the person's edit is not overwritten"); no_card(&state,&room);
}


#[tokio::test]
async fn an_unrepresentable_sub_cent_raise_is_never_rounded_to_a_broader_grant() {
    let service=service(2,10_000_000).await;
    let (state,token,room,_worktree)=super::without_asking_tests::agent_kernel(service.api_url.clone(),no_ask(None,None)).await;
    let error=change(&state,&token,serde_json::json!({"spend_per_day_micros":10_000_001})).await.unwrap_err();
    assert!(matches!(&error,DaemonError::MachineRequest {code,..} if code=="owner_action_required"),"{error}");
    let message=error.to_string();
    assert!(message.contains("10000001") || message.contains("10.000001"),"the full amount is named: {message}");
    assert!(!message.contains("/sudo raise"),"no rounded grant is invented: {message}");
    assert!(service.sets.lock().unwrap().is_empty()); assert_eq!(service.account.lock().unwrap().spend,10_000_000); no_card(&state,&room);
}


#[test]
fn named_limit_refusals_keep_the_exact_amount_without_floating_point_rounding() {
    assert_eq!(limits::amount_text(10_000_000,"EUR"),"10.00 EUR");
    assert_eq!(limits::amount_text(10_000_001,"EUR"),"10.000001 EUR");
    assert_eq!(limits::amount_text(15_010_000,"EUR"),"15.01 EUR");
    assert_eq!(limits::amount_text(u64::MAX,"EUR"),"18446744073709.551615 EUR");
}
