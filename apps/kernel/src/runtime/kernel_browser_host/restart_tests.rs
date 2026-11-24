//! Row 548: restart the user's browser. The controller process ends and a fresh one starts; nothing keeps the browser down.
use super::*;
use serde_json::json;

const CONTROLLER: &str = r#"set -eu
root=$1
printf 'boot\n' >> "$root/boots"
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"op":"stop"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","tabs":[]}}\n' "$id" ;;
 *'"method":"host.browser"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"},{"tab_id":"host-tab-b","document_id":"e","opener_tab_id":"host-tab-a"}]}}\n' "$id" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#;

fn host() -> (KernelBrowserHost, std::path::PathBuf) {
    let root =
        std::env::temp_dir().join(format!("chariox-restart-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, CONTROLLER).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("alice", &script, &root);
    (host, root)
}

fn request(host: &KernelBrowserHost, op: &str) -> Result<Value, String> {
    host.protected_request(
        "alice",
        None,
        "host.browser",
        json!({"op": op}),
        json!({"values":[],"targets":[],"unknown":false}),
    )
}

fn boots(root: &std::path::Path) -> usize {
    std::fs::read_to_string(root.join("boots")).map_or(0, |text| text.lines().count())
}

#[test]
fn restart_starts_a_fresh_controller_and_answers_with_the_restored_tabs() {
    let (host, root) = host();
    request(&host, "start").unwrap();
    assert_eq!(boots(&root), 1);
    let restarted = request(&host, "restart").unwrap();
    assert_eq!(boots(&root), 2, "the controller process was replaced");
    assert_eq!(restarted["tabs"][1]["opener_tab_id"], "host-tab-a");
    // Nothing keeps the browser down: an observation after a restart is served, as it is not after a stop.
    request(&host, "state").unwrap();
    assert_eq!(boots(&root), 2);
    host.shutdown().ok();
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn restart_after_a_stop_brings_the_browser_back_where_state_alone_cannot() {
    let (host, root) = host();
    request(&host, "start").unwrap();
    request(&host, "stop").unwrap();
    let refused = request(&host, "state").unwrap_err();
    assert!(refused.contains("browser_unavailable"), "{refused}");
    request(&host, "restart").unwrap();
    request(&host, "state").unwrap();
    host.shutdown().ok();
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn restart_of_a_browser_that_never_started_just_starts_it() {
    let (host, root) = host();
    let started = request(&host, "restart").unwrap();
    assert_eq!(boots(&root), 1);
    assert_eq!(started["tabs"][0]["tab_id"], "host-tab-a");
    host.shutdown().ok();
    std::fs::remove_dir_all(root).ok();
}
