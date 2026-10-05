//! MP-08/MP-10/MP-11: text waits share rendered paging rather than compact DOM text.
use super::observation_tests::runtime_with_room;
use super::*;

#[tokio::test]
async fn mp08_text_wait_reads_current_readonly_pane_and_preserves_query_bounds() {
    use crate::runtime::browser_controller_process::BrowserControllerProcessStore;
    let (root, mut runtime, room, agent) = runtime_with_room();
    let script = root.path().join("text-controller.cjs");
    let fixture = root.path().join("public-text.json");
    let text_module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/docker/browser-controller-text.mjs");
    std::fs::write(&script, r#"
const fs = require('node:fs');
const {pathToFileURL} = require('node:url');
const pager = import(pathToFileURL(process.argv[3]).href);
let revision = 0;
require('node:readline').createInterface({input: process.stdin}).on('line', async line => {
  const {id, method, params} = JSON.parse(line);
  let result;
  if (method === 'health') result = {state: 'ready', process_id: process.pid};
  else if (method === 'browser.reconcile') result = {
    browser_generation: 1,
    tabs: [{target_id: 'target-1', document_id: 'document-1', url: 'https://example.test', title: 'Example'}],
    focused_target_id: 'target-1', viewport: params.viewport,
    resource_inventory: {browser_ids: ['browser-fixture'], profile_ids: ['profile-fixture']},
  };
  else if (method === 'browser.snapshot') {
    const text = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
    result = {
      browser_generation: 1, target_id: params.target_id, document_id: params.document_id,
      snapshot_revision: ++revision, accessibility_nodes: [],
      dom_documents: [{document_index: 0, url: 'https://example.test', owner_node_ref: null}],
      dom_nodes: [{node_ref: 'backend:1', parent_ref: null, document_index: 0,
        node_type: 1, node_name: 'TEXTAREA', text: '', rendered: true,
        attributes: {readonly: ''}, bounds: {x: 0, y: 0, width: 100, height: 40}}],
    };
    if (params.text_request) result.text_page = (await pager).browserTextPage(text, params.text_request);
  } else if (method === 'shutdown') result = {state: 'stopped', process_id: null};
  else throw Error('unexpected public fixture method: ' + method);
  process.stdout.write(JSON.stringify({id, ok: true, result}) + '\n', () => {
    if (method === 'shutdown') process.exit(0);
  });
});
"#).unwrap();
    std::fs::write(&fixture, "\"Ready current value\"").unwrap();
    runtime.owned.browser_controller_processes = BrowserControllerProcessStore::new(
        "node",
        vec![
            script.display().to_string(),
            fixture.display().to_string(),
            text_module.display().to_string(),
        ],
        std::time::Duration::from_secs(2),
    );
    runtime
        .owned
        .browser_controller_processes
        .acquire(&room)
        .unwrap();
    let text = runtime
        .dispatch_room_browser_controller_runtime_tool_call(
            &room,
            "slice-test",
            &agent,
            crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL,
            serde_json::json!({"query": "Ready"}),
        )
        .await;
    let long_query = "日本".repeat(350);
    let cases = [
        (
            "current value",
            "Ready current value".to_string(),
            "Ready".to_string(),
            true,
        ),
        (
            "initial value excluded",
            "Ready current value".into(),
            "Old authored row".into(),
            false,
        ),
        (
            "redaction retained",
            "[redacted]".into(),
            "synthetic-protected-pane-value".into(),
            false,
        ),
        (
            "no editable or hidden fallback",
            "Ready current value".into(),
            "Editable draft".into(),
            false,
        ),
        (
            "match beyond first page",
            format!("{}\nReady", "prefix".repeat(400)),
            "Ready".into(),
            true,
        ),
        (
            "multiline across page",
            format!("{}one\ntwo", "x".repeat(1020)),
            "one\ntwo".into(),
            true,
        ),
        (
            "query above text-page limit",
            format!("prefix{long_query}suffix"),
            long_query,
            true,
        ),
        ("empty query", "".into(), "".into(), true),
    ];
    let mut results = Vec::new();
    for (label, rendered, query, expected) in cases {
        std::fs::write(&fixture, serde_json::to_vec(&rendered).unwrap()).unwrap();
        let result = runtime
            .dispatch_room_browser_controller_runtime_tool_call(
                &room,
                "slice-test",
                &agent,
                crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL,
                serde_json::json!({"text": query, "timeout_ms": 100}),
            )
            .await;
        results.push((label, result, expected));
    }
    runtime
        .owned
        .browser_controller_processes
        .release(&room)
        .unwrap();
    assert!(text
        .unwrap()
        .payload
        .to_string()
        .contains("Ready current value"));
    for (label, result, expected) in results {
        assert_eq!(result.unwrap().ok, expected, "MP-08/MP-10/MP-11 {label}");
    }
}
