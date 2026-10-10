//! MP-08/MP-11: authenticated agents open shared tabs, terminal inventory watches them.
use super::*;
use serde_json::json;
#[test]
fn mp08_delayed_popup_inventory_preserves_completed_agent_action_attribution() {
    let root = std::env::temp_dir().join(format!(
        "chariox-am6-delayed-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
acted=0
action=
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"op":"input"'*) acted=1; action=${request#*'"_action_id":"'}; action=${action%%'"'*}; printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"other","document_id":"other"}]}}\n' "$id" ;;
 *'"method":"host.browser"'*)
 if [ "$acted" = 1 ]; then
 printf '{"id":%s,"ok":true,"result":{"generation":1,"_tab_creation_actions":{"popup":"%s"},"_tab_openers":{"native":"source","other-popup":"other"},"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"other","document_id":"other"},{"tab_id":"popup","document_id":"popup"},{"tab_id":"native","document_id":"native"},{"tab_id":"other-popup","document_id":"other-popup"}]}}\n' "$id" "$action"
 else
 printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"other","document_id":"other"}]}}\n' "$id"
 fi ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("alice", &script, &root);
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    host.set_focus("alice", Some("mara"));
    host.load("alice", "mara").unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    let action = host.protected_request("alice", Some("mara"), "host.browser", json!({"op":"input","tab_id":"source","generation":1,"document_id":"doc","input":{"kind":"click","x":10,"y":10},"_action_id":"caller-forged"}), policy.clone()).unwrap();
    assert_eq!(action["tabs"].as_array().unwrap().len(), 2);
    let before = host.actor_snapshot("alice").unwrap()["input_ownership"].clone();
    let first = host
        .protected_request(
            "alice",
            None,
            "host.browser",
            json!({"op":"state","observed_by":"terminal:one"}),
            policy.clone(),
        )
        .unwrap();
    let second = host
        .protected_request(
            "alice",
            None,
            "host.browser",
            json!({"op":"state","observed_by":"terminal:two"}),
            policy,
        )
        .unwrap();
    let after = host.actor_snapshot("alice").unwrap()["input_ownership"].clone();
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    for state in [&first, &second] {
        assert!(state.get("_tab_openers").is_none());
        assert!(state.get("_tab_creation_actions").is_none());
        assert_eq!(state["tabs"][2]["opened_by"]["actor_id"], "agent:mara");
        assert!(state["tabs"][3]["opened_by"].is_null());
        assert!(state["tabs"][4]["opened_by"].is_null());
        assert_eq!(state["agent_activity"]["tab_id"], "popup");
    }
    assert_eq!(first["agent_activity"], second["agent_activity"]);
    assert_eq!(before, after);
}

#[test]
fn mp08_every_viewer_sees_agent_opener_and_activity_without_taking_control() {
    let root =
        std::env::temp_dir().join(format!("chariox-am6-tabs-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"method":"host.browser"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","generation":1,"tab_id":"host-agent","_tab_openers":{"unrelated":"host-agent"},"tabs":[{"tab_id":"host-agent","document_id":"doc","url":"https://developer.mozilla.org","title":"MDN"}]}}\n' "$id" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("alice", &script, &root);
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    host.set_focus("alice", Some("mara"));
    host.load("alice", "mara").unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    host.protected_request(
        "alice",
        Some("mara"),
        "host.browser",
        json!({"op":"open","url":"https://developer.mozilla.org"}),
        policy.clone(),
    )
    .unwrap();
    let before = host.actor_snapshot("alice").unwrap()["input_ownership"].clone();
    let mut receipts = Vec::new();
    for viewer in ["terminal:one", "terminal:two"] {
        let result = host
            .protected_request(
                "alice",
                None,
                "host.browser",
                json!({"op":"state","observed_by":viewer}),
                policy.clone(),
            )
            .unwrap();
        receipts.push(result);
    }
    let after = host.actor_snapshot("alice").unwrap()["input_ownership"].clone();
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    for result in receipts {
        assert!(result.get("_tab_openers").is_none());
        assert_eq!(result["tabs"][0]["opened_by"]["actor_id"], "agent:mara");
        assert_eq!(result["tabs"][0]["opened_by"]["display_label"], "mara");
        assert_eq!(result["tabs"][0]["title"], "MDN");
        assert_eq!(result["tabs"][0]["url"], "https://developer.mozilla.org");
        assert_eq!(result["agent_activity"]["tab_id"], "host-agent");
        assert_eq!(after, before);
    }
}
