//! MP-08/MP-09/MP-10/MP-11: dialog replies release the page action that opened them.
use super::*;
use serde_json::json;
use std::time::Instant;

#[test]
fn mp10_dialog_answer_is_admitted_directly_after_orphaned_takeover_expires() {
    assert_admitted_after_orphaned_takeover_expires(json!({
        "op":"dialog", "dialog_id":"current-dialog", "action":"accept"
    }));
}

#[test]
fn mp10_download_cancellation_is_admitted_directly_after_orphaned_takeover_expires() {
    assert_admitted_after_orphaned_takeover_expires(json!({
        "op":"cancel_download", "download_id":"download-a"
    }));
}

fn assert_admitted_after_orphaned_takeover_expires(mut params: Value) {
    let root = crate::test_support::TestWorktree::new("mp10-dialog-orphan-expiry");
    let script = root.path().join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
state='{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}'
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"method":"host.browser"'*)
  printf '%s\n' "$request" >> "$root/requests"
  printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$state" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null}}\n' "$id"; exit 0 ;;
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
    host.set_focus("alice", Some("agent"));
    host.load("alice", "agent").unwrap();
    let admission = host.admit("alice", "agent").unwrap();
    host.claim_resource(
        Some(&admission),
        UserDomainResource::BrowserTab {
            tab_id: "host-tab-a".into(),
        },
        false,
    )
    .unwrap();
    let viewer = host.lease_terminal("alice", "terminal:human");
    host.request_takeover(
        "alice",
        EnvironmentActor::new("terminal:human", EnvironmentActorKind::Human, "Human"),
        "host-tab-a",
        1,
    )
    .unwrap();
    params["tab_id"] = "host-tab-a".into();
    params["generation"] = 1.into();
    params["document_id"] = "d".into();
    let request = || {
        host.protected_request_admitted(
            "alice",
            Some(&admission),
            "host.browser",
            params.clone(),
            policy.clone(),
        )
    };
    let requests_before = std::fs::read_to_string(root.path().join("requests")).unwrap();
    let active = request();
    drop(viewer);
    let recent_orphan = request();
    assert_eq!(
        std::fs::read_to_string(root.path().join("requests")).unwrap(),
        requests_before,
        "takeover refusals must not reach the controller"
    );
    // Advance only the deadline: no state read or reclamation before the request under test.
    host.actor_model("alice")
        .unwrap()
        .lock()
        .unwrap()
        .elapse_orphaned_takeovers();
    let expired = request();
    let requests_after = std::fs::read_to_string(root.path().join("requests")).unwrap();
    host.shutdown().unwrap();
    for refused in [active, recent_orphan] {
        assert!(
            matches!(
                refused,
                Err(crate::error::HostFailure::Refused(
                    crate::error::UserDomainRefusalReason::NotGranted
                ))
            ),
            "live takeover or unexpired orphan must fence the agent: {refused:?}"
        );
    }
    assert!(
        expired.is_ok(),
        "{} must reclaim the expired takeover before admission: {expired:?}",
        params["op"]
    );
    let sent: Vec<Value> = requests_after
        .lines()
        .skip(requests_before.lines().count())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        sent.len(),
        1,
        "the operation must reach the controller directly"
    );
    assert_eq!(sent[0]["params"]["op"], params["op"]);
}

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

#[test]
fn dialog_answer_honors_human_takeover_without_blocking_human_answers() {
    let root = crate::test_support::TestWorktree::new("browser-dialog-takeover");
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

    host.set_focus("alice", Some("agent"));
    host.load("alice", "agent").unwrap();
    let admission = host.admit("alice", "agent").unwrap();
    host.claim_resource(
        Some(&admission),
        UserDomainResource::BrowserTab {
            tab_id: "host-tab-a".into(),
        },
        false,
    )
    .unwrap();
    host.request_takeover(
        "alice",
        EnvironmentActor::new("terminal:human", EnvironmentActorKind::Human, "Human"),
        "host-tab-a",
        1,
    )
    .unwrap();
    let params = json!({"op":"dialog","tab_id":"host-tab-a","generation":1,"document_id":"d","dialog_id":"current-dialog","action":"accept","observed_by":"terminal:human"});
    let refused = host.protected_request_admitted(
        "alice",
        Some(&admission),
        "host.browser",
        params.clone(),
        policy.clone(),
    );
    let cancel = json!({"op":"cancel_download","tab_id":"host-tab-a","generation":1,"download_id":"download-a"});
    assert!(matches!(
        host.protected_request_admitted(
            "alice",
            Some(&admission),
            "host.browser",
            cancel,
            policy.clone()
        ),
        Err(crate::error::HostFailure::Refused(
            crate::error::UserDomainRefusalReason::NotGranted
        ))
    ));
    let reached = root.path().join("answered").exists();
    let human = host.protected_request(
        "alice",
        None,
        "host.browser",
        params.clone(),
        policy.clone(),
    );
    host.release_input("alice", "terminal:human", "host-tab-a", 1)
        .unwrap();
    let agent =
        host.protected_request_admitted("alice", Some(&admission), "host.browser", params, policy);
    let activity = host.actor_snapshot("alice").unwrap()["agent_activity"].clone();
    host.shutdown().unwrap();
    assert!(matches!(
        refused,
        Err(crate::error::HostFailure::Refused(
            crate::error::UserDomainRefusalReason::NotGranted
        ))
    ));
    assert!(!reached, "refused answer must not reach the controller");
    assert!(
        human.is_ok(),
        "human answers remain unrestricted: {human:?}"
    );
    assert!(agent.is_ok(), "release restores agent answers: {agent:?}");
    assert_eq!(activity["actor"]["actor_id"], "agent:agent");
}
