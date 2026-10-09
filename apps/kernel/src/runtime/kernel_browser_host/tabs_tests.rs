//! MP-08/MP-11: authenticated agents open shared tabs, terminal inventory watches them.
use super::*;
use serde_json::json;
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
 *'"method":"host.browser"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","generation":1,"tab_id":"host-agent","tabs":[{"tab_id":"host-agent","document_id":"doc","url":"https://developer.mozilla.org","title":"MDN"}]}}\n' "$id" ;;
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
        assert_eq!(result["tabs"][0]["opened_by"]["actor_id"], "agent:mara");
        assert_eq!(result["tabs"][0]["opened_by"]["display_label"], "mara");
        assert_eq!(result["tabs"][0]["title"], "MDN");
        assert_eq!(result["tabs"][0]["url"], "https://developer.mozilla.org");
        assert_eq!(result["agent_activity"]["tab_id"], "host-agent");
        assert_eq!(after, before);
    }
}
