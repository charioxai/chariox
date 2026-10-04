//! MP-08/MP-10/MP-11: fail-first public Room dialog fixture and admission fences.
use super::observation_tests::runtime_with_room;
use super::*;
use crate::session::EnvironmentActionState;

#[tokio::test]
async fn mp08_dialog_tool_answers_without_renderer_reconcile() {
    use crate::runtime::browser_controller_process::BrowserControllerProcessStore;
    let (root, mut runtime, room, agent) = runtime_with_room();
    let script = root.path().join("dialog-controller.cjs");
    let calls = root.path().join("calls.jsonl");
    std::fs::write(
        &script,
        r#"
const fs = require('node:fs');
require('node:readline').createInterface({input: process.stdin}).on('line', line => {
  const {id, method, params} = JSON.parse(line);
  fs.appendFileSync(process.argv[2], JSON.stringify({method, params}) + '\n');
  let result;
  if (method === 'health') result = {state: 'ready', process_id: process.pid};
  else if (method === 'browser.dialog') result = {
browser_generation: 1, target_id: params.target_id, document_id: params.document_id,
action: params.action,
  };
  else if (method === 'shutdown') result = {state: 'stopped', process_id: null};
  else {
process.stdout.write(JSON.stringify({id, ok: false, error: {
  code: 'browser_cdp_timeout', message: 'Page.getFrameTree blocked by dialog',
}}) + '\n');
return;
  }
  process.stdout.write(JSON.stringify({id, ok: true, result}) + '\n', () => {
if (method === 'shutdown') process.exit(0);
  });
});
"#,
    )
    .unwrap();
    runtime.owned.browser_controller_processes = BrowserControllerProcessStore::new(
        "node",
        vec![script.display().to_string(), calls.display().to_string()],
        std::time::Duration::from_secs(2),
    );
    runtime
        .owned
        .browser_controller_processes
        .acquire(&room)
        .unwrap();
    let call = |agent: String| {
        let runtime = runtime.clone();
        let room = room.clone();
        async move {
            runtime
                .dispatch_room_browser_controller_runtime_tool_call(
                    &room,
                    "slice-test",
                    &agent,
                    crate::transport::runtime_tools::SLICE_BROWSER_DIALOG_TOOL,
                    serde_json::json!({"action": "accept", "prompt_text": "fixture answer"}),
                )
                .await
        }
    };
    let foreign = call("foreign-agent".into()).await.unwrap_err();
    assert!(foreign.to_string().contains("environment_unknown_actor"));
    let tab = runtime
        .room_environment_snapshot(&room)
        .unwrap()
        .focused_tab_id
        .unwrap();
    let target = crate::session::InputTarget::BrowserTab(tab);
    let human = crate::session::EnvironmentActor::new(
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID),
        crate::session::EnvironmentActorKind::Human,
        "Operator",
    );
    runtime
        .request_room_environment_takeover_as_actor(&room, human.clone(), target.clone())
        .unwrap();
    let takeover = call(agent.clone()).await.unwrap_err();
    assert!(takeover.to_string().contains("belongs to"));
    runtime
        .release_room_environment_input(&room, &human.actor_id, &target)
        .unwrap();
    let result = runtime
        .dispatch_room_browser_controller_runtime_tool_call(
            &room,
            "slice-test",
            &agent,
            crate::transport::runtime_tools::SLICE_BROWSER_DIALOG_TOOL,
            serde_json::json!({"action": "accept", "prompt_text": "fixture answer"}),
        )
        .await;
    runtime
        .owned
        .browser_controller_processes
        .release(&room)
        .unwrap();
    result.expect("MP-08/MP-10/MP-11 dialog answer must reach the paused renderer");
    let records: Vec<serde_json::Value> = std::fs::read_to_string(calls)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!records.iter().any(|r| r["method"] == "browser.reconcile"));
    let answers: Vec<_> = records
        .iter()
        .filter(|r| r["method"] == "browser.dialog")
        .collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["params"]["target_id"], "target-1");
    assert_eq!(answers[0]["params"]["document_id"], "document-1");
    let history = runtime
        .room_environment_action_history(&room, None, 100)
        .unwrap();
    let dialogs: Vec<_> = history
        .actions
        .iter()
        .filter(|a| a.kind == "dialog")
        .collect();
    assert_eq!(dialogs.len(), 1);
    assert_eq!(dialogs[0].state, EnvironmentActionState::Completed);
}
