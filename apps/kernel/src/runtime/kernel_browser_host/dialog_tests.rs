//! MP-08/MP-09/MP-10/MP-11: dialog replies release the page action that opened them.
use super::*;
use serde_json::json;
use std::time::Instant;

#[test]
fn dialog_answer_does_not_wait_for_the_page_action_lane() {
    let root = crate::test_support::TestWorktree::new("browser-dialog-lane");
    let script = root.path().join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
state='{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}'
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"op":"input"'*)
  printf started > "$root/started"
  (while [ ! -f "$root/release" ]; do sleep 0.01; done
   printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$state") & ;;
 *'"op":"dialog"'*)
  printf answered > "$root/answered"
  printf release > "$root/release"
  printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$state" ;;
 *'"method":"host.browser"'*) printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$state" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.path().to_path_buf());
    host.install_fixture_backend("alice", &script, root.path());
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    host.protected_request(
        "alice",
        None,
        "host.browser",
        json!({"op":"state"}),
        policy.clone(),
    )
    .unwrap();
    let result = std::thread::scope(|scope| {
        let input = scope.spawn(|| host.protected_request("alice", None, "host.browser",
            json!({"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"d","observed_by":"terminal:first","input":{"kind":"click","x":1,"y":1}}), policy.clone()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.path().join("started").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let entered = root.path().join("started").exists();
        let answer = host.protected_request("alice", None, "host.browser",
            json!({"op":"dialog","tab_id":"host-tab-a","generation":1,"document_id":"d","dialog_id":"current-dialog","action":"accept","observed_by":"terminal:second"}), policy.clone());
        // Settle the input even when the old arbitration refuses the answer.
        std::fs::write(root.path().join("release"), "release").unwrap();
        input.join().unwrap().unwrap();
        (entered, answer, root.path().join("answered").exists())
    });
    host.shutdown().unwrap();
    assert!(result.0, "page action did not enter the controller");
    assert!(
        result.1.is_ok(),
        "dialog reply was refused behind page input: {:?}",
        result.1
    );
    assert!(result.2, "dialog reply never reached the controller");
}
