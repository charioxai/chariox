//! MP-08/MP-10: synthetic Room actors use the real kernel and headed controller.
//! Opt-in runner owns Docker, page gates, evidence and cleanup. No provider mocks.
use super::*;
use crate::runtime::browser_controller_action::BrowserLocatorAction;
use crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait;
use crate::runtime::browser_controller_process::BrowserControllerProcessStore;
use crate::session::{EnvironmentActionState, EnvironmentActor, EnvironmentActorKind, InputTarget};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(path: &str) -> serde_json::Value {
    let address = std::env::var("CHARIOX_CONCURRENCY_FIXTURE").unwrap();
    let mut socket = tokio::net::TcpStream::connect(&address).await.unwrap();
    socket
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let body = String::from_utf8(bytes).unwrap();
    assert!(
        body.starts_with("HTTP/1.1 200"),
        "MP-08/MP-10 fixture HTTP failed"
    );
    serde_json::from_str(body.split_once("\r\n\r\n").unwrap().1).unwrap()
}

async fn until(mut condition: impl AsyncFnMut() -> bool, label: &str) {
    let start = Instant::now();
    while !condition().await {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "MP-08/MP-10 {label} did not reach the physical seam within 3s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn control(query: &str) -> u64 {
    let epoch = fixture(&format!("/control?{query}")).await["epoch"]
        .as_u64()
        .unwrap();
    let epoch_text = epoch.to_string();
    until(
        async || {
            let state = fixture("/state").await;
            ["same", "other"].into_iter().all(|tab| {
                state["probes"].as_array().unwrap().iter().any(|p| {
                    p["epoch"].as_str() == Some(epoch_text.as_str()) && p["kind"] == "ready" && p["tab"] == tab
                })
            })
        },
        "page gate acknowledgment",
    )
    .await;
    epoch
}

fn history(runtime: &KernelRuntimeState, room: &str) -> Vec<crate::session::EnvironmentAction> {
    runtime
        .room_environment_action_history(room, None, 100)
        .unwrap()
        .actions
}

async fn read(
    runtime: &KernelRuntimeState,
    room: &str,
    agent: &str,
    tab: &EnvironmentTab,
    selector: &str,
) {
    // The same production observation admission/finalization used by status/find.
    let environment = ensure_controller_browser_environment(runtime, room, "concurrency.read")
        .await
        .unwrap();
    let id = begin_controller_browser_observation(
        runtime,
        room,
        agent,
        &environment,
        tab,
        "browser_status",
    )
    .unwrap();
    let guard = Some(ControllerBrowserObservationGuard::new(
        runtime,
        room,
        id,
        "browser_status",
    ));
    let binding = runtime
        .room_environment_controller_tab_binding(room, &tab.tab_id)
        .unwrap();
    let (_, mut observation) = await_controller_browser_observation(guard, async {
        let result = runtime
            .room_browser_controller_command(
                room,
                Command::Wait {
                    target_id: binding.runtime_target_id,
                    document_id: binding.document_id,
                    wait: BrowserCompatibilityWait::Selector(selector.into()),
                    timeout_ms: 5000,
                },
            )
            .await?;
        assert!(matches!(result, Response::Wait { result: Some(_) }));
        Ok::<_, DaemonError>(())
    })
    .await
    .unwrap();
    observation
        .as_mut()
        .unwrap()
        .finish(EnvironmentActionTerminal::Completed)
        .unwrap();
}

async fn mutation(
    runtime: &KernelRuntimeState,
    room: &str,
    agent: &str,
    element: &str,
    action: BrowserLocatorAction,
) -> Result<String, DaemonError> {
    runtime
        .perform_browser_environment_locator_action_as_agent(room, agent, element, action, 5000)
        .await
        .map(|r| r.action_id)
}

fn reference(
    snapshot: &crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot,
    role: &str,
    name: &str,
) -> String {
    snapshot
        .accessibility_nodes
        .iter()
        .find(|n| n.role == role && n.name == name)
        .unwrap()
        .element_ref
        .clone()
}

fn interval_overlap(actions: &[crate::session::EnvironmentAction]) -> u64 {
    let start = actions
        .iter()
        .map(|a| a.started_at_ms.unwrap())
        .max()
        .unwrap();
    let end = actions
        .iter()
        .map(|a| a.finished_at_ms.unwrap())
        .min()
        .unwrap();
    assert!(
        end > start,
        "MP-08/MP-10 physical cohort had no Action interval overlap"
    );
    end - start
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "MP-08/MP-10 run through live-browser-controller-concurrency-drill.mjs; requires owned headed slice"]
async fn headed_controller_concurrency_acceptance() {
    let container = std::env::var("CHARIOX_CONCURRENCY_CONTAINER").unwrap();
    let (_root, mut runtime, room, agent_a) = super::observation_tests::runtime_with_room();
    let agent_b = runtime
        .with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(&room, "dev-stub").with_alias("reader-b"),
                )
                .unwrap()
                .id()
                .to_string()
        })
        .await;
    let agent_c = runtime
        .with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(&room, "dev-stub").with_alias("worker-c"),
                )
                .unwrap()
                .id()
                .to_string()
        })
        .await;
    runtime.set_browser_controller_process_store_for_test(BrowserControllerProcessStore::new(
        "docker",
        vec![
            "exec".into(),
            "-i".into(),
            "-u".into(),
            "slice".into(),
            container,
            "node".into(),
            "/opt/chariox-slice/browser-controller.mjs".into(),
            "stdio".into(),
        ],
        Duration::from_secs(10),
    ));
    let result = std::panic::AssertUnwindSafe(run(&runtime, &room, [&agent_a, &agent_b, &agent_c]))
        .catch_unwind()
        .await;
    runtime
        .stop_browser_controller_process(&room)
        .await
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

use futures_util::FutureExt;

async fn run(runtime: &KernelRuntimeState, room: &str, agents: [&str; 3]) {
    runtime
        .ensure_browser_controller_process_started(room)
        .await
        .unwrap();
    let environment = runtime
        .reconcile_browser_controller_environment(room)
        .await
        .unwrap();
    let same = environment
        .tabs
        .iter()
        .find(|t| t.title == "MP-08/MP-10 same")
        .unwrap()
        .clone();
    let other = environment
        .tabs
        .iter()
        .find(|t| t.title == "MP-08/MP-10 other")
        .unwrap()
        .clone();
    let same_snapshot = runtime
        .capture_browser_environment_snapshot(room, &same.tab_id)
        .await
        .unwrap();
    let other_snapshot = runtime
        .capture_browser_environment_snapshot(room, &other.tab_id)
        .await
        .unwrap();
    let same_button = reference(&same_snapshot, "button", "Click same");
    let other_button = reference(&other_snapshot, "button", "Click other");
    let same_note = reference(&same_snapshot, "textbox", "Note same");
    let other_note = reference(&other_snapshot, "textbox", "Note other");
    let focused = runtime
        .manage_browser_environment_tab(
            room,
            "ffffffffffffffffffffffffffffff01",
            &same.tab_id,
            crate::runtime::browser_controller_tab::BrowserTabAction::Activate,
        )
        .await
        .unwrap();
    let initial_focus = focused.focused_tab_id;

    // Real selector waits stay pending at a page gate; query probes acknowledge
    // all three CDP reads before release. No sleeps manufacture an overlap.
    let epoch = control("").await;
    let epoch_text = epoch.to_string();
    let reads = async {
        tokio::join!(
            read(runtime, room, agents[0], &same, "#read-a"),
            read(runtime, room, agents[1], &same, "#read-b"),
            read(runtime, room, agents[2], &other, "#read-c")
        );
    };
    let gate = async {
        until(
            async || {
                let state = fixture("/state").await;
                ["#read-a", "#read-b", "#read-c"].into_iter().all(|id| {
                    state["probes"].as_array().unwrap().iter().any(|p| {
                        p["epoch"].as_str() == Some(epoch_text.as_str()) && p["kind"] == "read" && p["id"] == id
                    })
                })
            },
            "three concurrent physical reads",
        )
        .await;
        assert_eq!(
            history(runtime, room)
                .iter()
                .filter(|a| a.state == EnvironmentActionState::Running)
                .count(),
            3
        );
        control("reads=release").await;
    };
    tokio::join!(reads, gate);
    let read_actions = history(runtime, room);
    assert_eq!(read_actions.len(), 3);
    let read_overlap = interval_overlap(&read_actions);

    // Held independent input must not drain read/status preflight on the other
    // tab. The status read must physically complete before the input is released.
    let epoch = control("other=hold").await;
    let epoch_text = epoch.to_string();
    let input = mutation(
        runtime,
        room,
        agents[2],
        &other_button,
        BrowserLocatorAction::Click,
    );
    let status_gate = async {
        until(
            async || {
                fixture("/state").await["probes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| {
                        p["epoch"].as_str() == Some(epoch_text.as_str())
                            && p["tab"] == "other"
                            && p["kind"] == "action"
                    })
            },
            "held background input",
        )
        .await;
        let started = Instant::now();
        let (a, b) = tokio::join!(
            run_controller_browser_status_tool(runtime, room, "synthetic", agents[0]),
            run_controller_browser_status_tool(runtime, room, "synthetic", agents[1])
        );
        a.unwrap();
        b.unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "MP-08/MP-10 independent status drained held input"
        );
        assert!(history(runtime, room)
            .iter()
            .any(|a| a.state == EnvironmentActionState::Running && a.kind == "click"));
        control("").await;
    };
    let (click, ()) = tokio::join!(input, status_gate);
    click.unwrap();

    // Two same-tab fills serialize; independent-tab fill overlaps the first.
    let epoch = control("same=hold&other=hold").await;
    let epoch_text = epoch.to_string();
    let fill = |text: &str| BrowserLocatorAction::Fill {
        text: text.into(),
        append: false,
        submit: false,
        expected_document_url: None,
    };
    let first = mutation(runtime, room, agents[0], &same_note, fill("first"));
    let queue_and_other = async {
        until(
            async || {
                history(runtime, room)
                    .iter()
                    .any(|a| a.state == EnvironmentActionState::Running && a.kind == "fill")
            },
            "first fill admission",
        )
        .await;
        tokio::join!(
            mutation(runtime, room, agents[1], &same_note, fill("second")),
            mutation(runtime, room, agents[2], &other_note, fill("independent"))
        )
    };
    let gate = async {
        until(
            async || {
                let actions = history(runtime, room);
                let state = fixture("/state").await;
                actions
                    .iter()
                    .filter(|a| a.kind == "fill" && a.state == EnvironmentActionState::Running)
                    .count()
                    == 2
                    && actions
                        .iter()
                        .filter(|a| a.kind == "fill" && a.state == EnvironmentActionState::Queued)
                        .count()
                        == 1
                    && ["same", "other"].into_iter().all(|tab| {
                        state["probes"].as_array().unwrap().iter().any(|p| {
                            p["epoch"].as_str() == Some(epoch_text.as_str())
                                && p["tab"] == tab
                                && p["kind"] == "action"
                                && p["id"] == "note"
                        })
                    })
            },
            "same-tab queued and independent physical fills",
        )
        .await;
        control("").await;
    };
    let (a, (b, c), ()) = tokio::join!(first, queue_and_other, gate);
    let [a, b, c] = [a.unwrap(), b.unwrap(), c.unwrap()];
    let actions = history(runtime, room);
    let action = |id: &str| actions.iter().find(|a| a.action_id == id).unwrap().clone();
    assert!(action(&b).started_at_ms.unwrap() >= action(&a).finished_at_ms.unwrap());
    let fill_overlap = interval_overlap(&[action(&a), action(&c)]);
    for (tab, value) in [(&same, "second"), (&other, "independent")] {
        let snapshot = runtime
            .capture_browser_environment_snapshot(room, &tab.tab_id)
            .await
            .unwrap();
        assert!(snapshot
            .accessibility_nodes
            .iter()
            .any(|n| n.role == "textbox" && n.value == value));
    }

    // Human takeover cancels exactly the held and queued same-tab operations.
    // Independent input remains live and completes exactly once after release.
    let epoch = control("same=hold&other=hold").await;
    let epoch_text = epoch.to_string();
    let first = mutation(
        runtime,
        room,
        agents[0],
        &same_button,
        BrowserLocatorAction::Click,
    );
    let rest = async {
        until(
            async || {
                history(runtime, room).iter().any(|a| {
                    a.state == EnvironmentActionState::Running
                        && a.actor_id == crate::session::agent_environment_actor_id(agents[0])
                })
            },
            "takeover first admission",
        )
        .await;
        tokio::join!(
            mutation(runtime, room, agents[1], &same_note, fill("must-not-land")),
            mutation(
                runtime,
                room,
                agents[2],
                &other_button,
                BrowserLocatorAction::Click
            )
        )
    };
    let human = EnvironmentActor::new("human:concurrency", EnvironmentActorKind::Human, "Operator");
    let takeover = async {
        until(
            async || {
                let actions = history(runtime, room);
                actions
                    .iter()
                    .filter(|a| a.state == EnvironmentActionState::Running)
                    .count()
                    == 2
                    && actions
                        .iter()
                        .any(|a| a.state == EnvironmentActionState::Queued)
                    && fixture("/state").await["probes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|p| {
                            p["epoch"].as_str() == Some(epoch_text.as_str())
                                && p["kind"] == "action"
                                && p["id"] == "button"
                        })
                        .count()
                        >= 2
            },
            "takeover physical cohort",
        )
        .await;
        let started = Instant::now();
        runtime
            .request_room_environment_takeover_as_actor(
                room,
                human.clone(),
                InputTarget::BrowserTab(same.tab_id.clone()),
            )
            .unwrap();
        until(
            async || {
                let env = runtime.room_environment_snapshot(room).unwrap();
                env.pending_input_takeovers.is_empty()
                    && env.input_ownership.iter().any(|owner| {
                        owner.actor_id == human.actor_id
                            && owner.target == InputTarget::BrowserTab(same.tab_id.clone())
                    })
            },
            "human ownership after cancellation",
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(history(runtime, room)
            .iter()
            .any(|a| a.state == EnvironmentActionState::Running
                && a.actor_id == crate::session::agent_environment_actor_id(agents[2])));
        control("").await;
    };
    let (a, (b, c), ()) = tokio::join!(first, rest, takeover);
    assert!(a.is_err());
    assert!(b.is_err());
    c.unwrap();
    let actions = history(runtime, room);
    let cancelled = actions
        .iter()
        .filter(|a| a.state == EnvironmentActionState::Cancelled)
        .collect::<Vec<_>>();
    assert_eq!(cancelled.len(), 2);
    assert!(
        cancelled.iter().any(|a| a.started_at_ms.is_none()),
        "MP-08/MP-10 queued takeover input was dispatched"
    );
    let after = runtime
        .reconcile_browser_controller_environment(room)
        .await
        .unwrap();
    assert_eq!(after.focused_tab_id, initial_focus);
    let snapshot = runtime
        .capture_browser_environment_snapshot(room, &same.tab_id)
        .await
        .unwrap();
    assert!(snapshot
        .accessibility_nodes
        .iter()
        .any(|n| n.role == "textbox" && n.value == "second"));
    let effects = fixture("/state").await;
    let click_count = |tab: &str| {
        effects["effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["tab"] == tab && e["kind"] == "click")
            .count()
    };
    assert_eq!(click_count("same"), 0);
    assert_eq!(click_count("other"), 2);
    for action in &actions {
        assert!(agents
            .iter()
            .any(|agent| action.actor_id == crate::session::agent_environment_actor_id(agent)));
        assert!(matches!(
            action.state,
            EnvironmentActionState::Completed | EnvironmentActionState::Cancelled
        ));
    }
    println!(
        "{}",
        serde_json::json!({"schema":"chariox.controller_concurrency.v1","mp_items":["MP-08","MP-10"],
        "status":"GREEN","readOverlapMs":read_overlap,"fillOverlapMs":fill_overlap,"actions":actions,"fixture":effects,
        "focusPreserved":true,"sameTabSerialized":true,"takeoverCancelled":2,"controllerBoundMs":5000})
    );
    runtime
        .release_room_environment_input(
            room,
            &human.actor_id,
            &InputTarget::BrowserTab(same.tab_id),
        )
        .unwrap();
}
