use super::computer_input_reconcile_test_support::{TestRoom, TestTools};
use super::*;
use crate::runtime::browser_controller_tab::BrowserTabAction;
use tokio::sync::oneshot;

#[tokio::test]
async fn controller_restart_keeps_queued_browser_execution_until_reconciliation() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("restart-queued-browser");
    let trigger = tools.controller_tool.with_file_name("restart-trigger");
    let recovering = tools.controller_tool.with_file_name("reconcile-started");
    let release = tools.controller_tool.with_file_name("reconcile-release");
    let effects = tools.controller_tool.with_file_name("tab-effects");
    let script = std::fs::read_to_string(&tools.controller_tool)
        .unwrap()
        .replace("const { id, method }", "const { id, method, params }")
        .replace("loader-after-input", "loader-before-input")
        .replace("https://after-input.test", "https://before-input.test")
        .replace("After input", "Before input")
        .replace("const { createInterface }", "const fs = require('node:fs');\nconst { createInterface }")
        .replace(
            "} else if (method === \"browser.reconcile\") {",
            &format!(r#"}} else if (method === "browser.tab") {{
    fs.appendFileSync({}, 'effect\n');
    result = {{browser_generation:1,target_id:params.target_id,document_id:params.document_id,action:params.action}};
  }} else if (method === "browser.reconcile") {{
    if (fs.existsSync({})) {{
      fs.writeFileSync({}, 'waiting');
      while (!fs.existsSync({})) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,10);
    }}"#,
                serde_json::to_string(&effects).unwrap(),
                serde_json::to_string(&trigger).unwrap(),
                serde_json::to_string(&recovering).unwrap(),
                serde_json::to_string(&release).unwrap()),
        );
    std::fs::write(&tools.controller_tool, script).unwrap();
    let mut room = TestRoom::new("restart-queued-browser");
    room.enable_browser_controller(&tools).await;
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let tab = before.tabs[0].clone();
    let process = room
        .runtime
        .owned
        .browser_controller_processes
        .snapshot()
        .unwrap()
        .unwrap();
    let command = |id: &str| RoomBrowserControllerCommand::Tab {
        execution_id: id.into(),
        target_id: "target-a".into(),
        document_id: "loader-before-input".into(),
        action: BrowserTabAction::Activate,
    };
    let (started_tx, started_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let first = room.runtime.execute_browser_mutation_as_agent(
        &room.session_id,
        &room.agent_id,
        &tab.tab_id,
        tab.document_revision,
        "restart-first",
        Some("00000000000000000000000000000001"),
        async {
            started_tx.send(()).unwrap();
            resume_rx.await.unwrap();
            std::fs::write(&trigger, "restart").unwrap();
            // Kill only our captured fixture controller; browser/document identity survives.
            assert_eq!(
                unsafe { libc::kill(process.process_id.unwrap() as i32, libc::SIGKILL) },
                0
            );
            let result = room
                .runtime
                .room_browser_controller_command(
                    &room.session_id,
                    command("00000000000000000000000000000001"),
                )
                .await;
            assert!(
                matches!(
                    &result,
                    Err(DaemonError::BrowserControllerRecoveryRequired { .. })
                ),
                "actual mutation must report the implicit controller restart: {result:?}"
            );
            result
        },
    );
    let second = async {
        started_rx.await.unwrap();
        room.runtime
            .execute_browser_mutation_as_agent(
                &room.session_id,
                &room.agent_id,
                &tab.tab_id,
                tab.document_revision,
                "restart-queued",
                Some("00000000000000000000000000000002"),
                room.runtime.room_browser_controller_command(
                    &room.session_id,
                    command("00000000000000000000000000000002"),
                ),
            )
            .await
    };
    let drive = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = room
                    .runtime
                    .room_environment_snapshot(&room.session_id)
                    .unwrap();
                if snapshot.actions.iter().any(|a| {
                    a.kind == "restart-queued" && a.state == EnvironmentActionState::Queued
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        resume_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !recovering.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let during = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap();
        std::fs::write(&release, "release").unwrap();
        during
    };
    let (first, second, during) = tokio::join!(first, second, drive);
    let after = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    assert!(first.is_err(), "interrupted action must not be replayed");
    assert_eq!(
        during
            .actions
            .iter()
            .find(|a| a.kind == "restart-queued")
            .unwrap()
            .state,
        EnvironmentActionState::Queued,
        "unstarted queued action must survive the recovery fence"
    );
    assert!(
        second.is_ok(),
        "queued action must execute after reconciliation: {second:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&effects).unwrap(),
        "effect\n",
        "only queued B executes, exactly once"
    );
    assert_eq!(after.runtime_generation, before.runtime_generation);
    assert_eq!(
        after.tabs, before.tabs,
        "controller-only restart preserves document identity"
    );
    assert_eq!(
        after
            .actions
            .iter()
            .find(|a| a.kind == "restart-queued")
            .unwrap()
            .state,
        EnvironmentActionState::Completed
    );
}

#[tokio::test]
async fn controller_restart_fence_blocks_promotion_by_other_terminal_actions() {
    crate::test_support::isolated_env_test!();
    let room = TestRoom::new("restart-computer-settlement");
    let snapshot = room
        .runtime
        .reconcile_room_environment_actors(&room.session_id, None)
        .unwrap();
    let actor = agent_environment_actor_id(&room.agent_id);
    let tab = &snapshot.tabs[0];
    let browser = || {
        EnvironmentActionRequest::browser_mutation(
            &actor,
            snapshot.runtime_generation,
            "click",
            &tab.tab_id,
            tab.document_revision,
        )
    };
    let (a, _) = room
        .runtime
        .submit_room_environment_action(&room.session_id, browser())
        .unwrap();
    assert!(matches!(a, ActionAdmission::Accepted { .. }));
    let (b, _) = room
        .runtime
        .submit_room_environment_action(&room.session_id, browser())
        .unwrap();
    let ActionAdmission::Queued { action_id: b, .. } = b else {
        panic!("Browser B must queue")
    };
    let computer = || {
        EnvironmentActionRequest::computer_mutation(
            &actor,
            snapshot.runtime_generation,
            "key",
            None,
        )
    };
    let (x, _) = room
        .runtime
        .submit_room_environment_action(&room.session_id, computer())
        .unwrap();
    let ActionAdmission::Accepted { action_id: x } = x else {
        panic!("Computer X must run")
    };
    let (y, _) = room
        .runtime
        .submit_room_environment_action(&room.session_id, computer())
        .unwrap();
    let ActionAdmission::Queued { action_id: y, .. } = y else {
        panic!("Computer Y must queue")
    };
    room.runtime
        .begin_room_environment_browser_controller_recovery(&room.session_id)
        .unwrap();
    let fenced = room
        .runtime
        .finish_room_environment_action(&room.session_id, &x, EnvironmentActionTerminal::Completed)
        .unwrap();
    for id in [&b, &y] {
        assert_eq!(
            fenced
                .actions
                .iter()
                .find(|a| &a.action_id == id)
                .unwrap()
                .state,
            EnvironmentActionState::Queued,
            "terminal Computer work must not bypass pending Browser reconciliation"
        );
    }
    let reconciled = room
        .runtime
        .complete_room_environment_browser_controller_recovery(&room.session_id)
        .unwrap();
    for id in [&b, &y] {
        assert_eq!(
            reconciled
                .actions
                .iter()
                .find(|a| &a.action_id == id)
                .unwrap()
                .state,
            EnvironmentActionState::Running
        );
    }
}
