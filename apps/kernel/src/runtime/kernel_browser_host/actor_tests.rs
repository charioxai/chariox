//! MD-3/MD-4: actual host ledger → controller cancellation, no Chromium dependency.
use super::*;
use serde_json::json;
#[test]
fn human_takeover_cancels_the_running_controller_and_fences_the_focused_agent() {
    let root = std::env::temp_dir().join(format!(
        "chariox-md3-takeover-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
    *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}}\n' "$id" ;;
    *'"op":"input"'*) pending=$id; printf 'started\n' > "$root/started" ;;
    *'"method":"browser.cancel"'*)
      printf 'cancelled\n' > "$root/cancelled"
      printf '{"id":%s,"ok":true,"result":{"accepted":true}}\n' "$id"
      printf '{"id":%s,"ok":false,"error":{"code":"browser_action_cancelled","message":"cancelled"}}\n' "$pending" ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#).unwrap();
    let backend = BrowserControllerProcessStdioBackend::new(
        "/bin/sh",
        vec![script.display().to_string(), root.display().to_string()],
        Duration::from_secs(2),
    )
    .for_host();
    let host = KernelBrowserHost::new(root.clone());
    host.inner
        .lock()
        .unwrap()
        .browsers
        .insert("alice".into(), Arc::new(Mutex::new(backend)));
    host.set_focus("alice", Some("agent"));
    host.load("alice", "agent").unwrap();
    let mutation = json!({"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"d","input":{"kind":"text","text":"must-not-be-retained"}});
    let policy = json!({"values":[],"targets":[],"unknown":false});
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            host.protected_request(
                "alice",
                Some("agent"),
                "host.browser",
                mutation.clone(),
                policy.clone(),
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !root.join("started").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("started").exists());
        let takeover = host
            .request_takeover(
                "alice",
                EnvironmentActor::new("terminal:a", EnvironmentActorKind::Human, "Alice"),
                "host-tab-a",
                1,
            )
            .unwrap();
        assert!(matches!(
            takeover,
            TakeoverOutcome::CancellationRequired { .. }
        ));
        assert!(host
            .request_takeover(
                "bob",
                EnvironmentActor::new("terminal:b", EnvironmentActorKind::Human, "Bob"),
                "host-tab-a",
                1
            )
            .is_err());
        assert!(running
            .join()
            .unwrap()
            .unwrap_err()
            .contains("browser_action_cancelled"));
    });
    assert!(root.join("cancelled").exists());
    let admission = host.admit("alice", "agent").unwrap();
    assert!(matches!(
        host.protected_request_admitted(
            "alice",
            Some(&admission),
            "host.browser",
            mutation,
            policy
        )
        .unwrap_err(),
        crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
    ));
    let state = host.actor_snapshot("alice").unwrap();
    assert_eq!(state["input_ownership"][0]["actor_id"], "terminal:a");
    assert_eq!(state["actions"][0]["state"], "cancelled");
    assert_eq!(state["actions"][0]["outcome"]["reason"], "human_takeover");
    assert!(!state.to_string().contains("must-not-be-retained"));
    assert!(host
        .release_input("alice", "terminal:b", "host-tab-a", 1)
        .is_err());
    host.release_input("alice", "terminal:a", "host-tab-a", 1)
        .unwrap();
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retired_agents_reclaim_actor_slots_beyond_the_presence_limit() {
    let host = KernelBrowserHost::new(std::env::temp_dir().join("MD-3-retired-actors"));
    for i in 0..80 {
        let agent = format!("retired-{i}");
        host.set_focus("alice", Some(&agent));
        host.load("alice", &agent).unwrap();
        let model = host.actor_model("alice").unwrap();
        let (action, _) = model
            .lock()
            .unwrap()
            .begin(
                EnvironmentActor::new(
                    format!("agent:{agent}"),
                    EnvironmentActorKind::Agent,
                    "Agent",
                ),
                &json!({"op":"open"}),
            )
            .unwrap();
        model
            .lock()
            .unwrap()
            .finish(&action, EnvironmentActionTerminal::Completed);
        // Retire a non-focused agent too: the prior focus interval is already revoked.
        host.set_focus("alice", None);
        host.revoke_agent(&agent);
        assert!(
            host.actor_snapshot("alice").unwrap()["actors"]
                .as_array()
                .unwrap()
                .is_empty(),
            "MD-3: retired presence consumes the next agent's slot"
        );
    }
    assert_eq!(
        host.actor_snapshot("alice").unwrap()["actions"]
            .as_array()
            .unwrap()
            .len(),
        80
    );
}

#[test]
fn disconnected_terminal_cannot_register_a_queued_mutation() {
    let root = std::env::temp_dir().join(format!(
        "chariox-md3-disconnect-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}}\n' "$id" ;;
  *'"op":"input"'*) printf 'input\n' > "$root/input"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let backend = Arc::new(Mutex::new(
        BrowserControllerProcessStdioBackend::new(
            "/bin/sh",
            vec![script.display().to_string(), root.display().to_string()],
            Duration::from_secs(2),
        )
        .for_host(),
    ));
    let host = KernelBrowserHost::new(root.clone());
    host.inner
        .lock()
        .unwrap()
        .browsers
        .insert("alice".into(), backend.clone());
    let held = backend.lock().unwrap();
    let lifetime = crate::runtime::command::TerminalLifetime::default();
    let admission = host.admit_terminal("alice", lifetime.clone());
    std::thread::scope(|scope| {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let host = &host;
        let admission = &admission;
        let queued = scope.spawn(move || {
            started_tx.send(()).unwrap();
            host.protected_request_admitted("alice", Some(admission), "host.browser", json!({"op":"input","tab_id":"host-tab-a","generation":1,"observed_by":"terminal:departed","input":{"kind":"text","text":"fixture"}}), json!({"values":[],"targets":[],"unknown":false}))
        });
        started_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        lifetime.cancel();
        host.disconnect_terminal("alice", "terminal:departed");
        drop(held);
        let outcome = queued.join().unwrap();
        assert!(
            outcome.is_err(),
            "MD-3: departed terminal survived the backend wait"
        );
    });
    assert!(
        !root.join("input").exists(),
        "MD-3: departed input reached the physical sink"
    );
    assert!(host.actor_snapshot("alice").unwrap()["actors"]
        .as_array()
        .unwrap()
        .is_empty());
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
