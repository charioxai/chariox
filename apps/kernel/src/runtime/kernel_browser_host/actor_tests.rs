//! MD-3/MD-4: actual host ledger → controller cancellation, no Chromium dependency.
use super::*;
use serde_json::json;
use std::time::Instant;
#[test]
fn human_takeover_cancels_the_running_controller_and_fences_the_focused_agent() {
    check_running_cancellation(false, false);
}
#[test]
fn mdaccess_revoke_cancels_inflight_controller_input_immediately() {
    check_running_cancellation(true, false);
}
#[test]
fn mdaccess_focus_change_cancels_focused_input_without_revoking_grant() {
    check_running_cancellation(false, true);
}
fn check_running_cancellation(revoke: bool, focus_change: bool) {
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
    *'"method":"host.revoke_subscriptions"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
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
        if revoke {
            host.revoke_agent("agent");
        } else if focus_change {
            host.set_focus("alice", Some("second"));
        } else {
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
        }
        let error = running.join().unwrap().unwrap_err();
        assert!(error.contains(if revoke {
            "revoked"
        } else {
            "browser_action_cancelled"
        }));
    });
    assert!(root.join("cancelled").exists());
    if revoke {
        assert!(!host.has_grant("alice", "agent"));
        assert!(host.actor_snapshot("alice").unwrap()["actors"]
            .as_array()
            .unwrap()
            .is_empty());
    } else if focus_change {
        assert!(
            host.has_grant("alice", "agent"),
            "MP-11: focus loss cancels only this focused input, not the retained grant"
        );
        assert!(host
            .check_admission(Some(&host.admit("alice", "agent").unwrap()))
            .is_ok());
    } else {
        assert!(host
            .protected_request("alice", Some("agent"), "host.browser", mutation, policy)
            .unwrap_err()
            .contains("human owns"));
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
    }
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
        // Retire a non-focused agent too: focus alone keeps the prior grant.
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

#[test]
fn mdaccess_revoke_releases_grant_lock_before_waiting_for_actor() {
    let host = KernelBrowserHost::new(std::env::temp_dir().join("mdaccess-lock-order"));
    host.set_focus("alice", Some("agent"));
    let epoch = host
        .inner
        .lock()
        .unwrap()
        .access
        .grant("alice", "agent")
        .unwrap()
        .epoch
        .clone();
    let model = host.actor_model("alice").unwrap();
    let actor_guard = model.lock().unwrap();
    let revoker = host.clone();
    let revoke = std::thread::spawn(move || revoker.revoke_agent("agent"));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !epoch.requested() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let reader = host.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let authority = std::thread::spawn(move || sender.send(reader.has_grant("alice", "agent")));
    let result = receiver.recv_timeout(Duration::from_secs(2));
    drop(actor_guard); // Also lets a broken implementation terminate cleanly.
    revoke.join().unwrap();
    authority.join().unwrap().unwrap();
    assert_eq!(
        result.unwrap(),
        false,
        "MP-11: revoke must not deadlock authority callbacks"
    );
}

#[test]
fn mdaccess_scoped_inventory_preserves_other_actor_tab_ownership() {
    let root = std::env::temp_dir().join(format!(
        "chariox-mdaccess-ledger-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"a","document_id":"doc-a"},{"tab_id":"b","document_id":"doc-b"}]}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("owner", &script, &root);
    host.set_focus("owner", Some("first"));
    host.load("owner", "first").unwrap();
    let admission = host.admit("owner", "first").unwrap();
    host.claim_resource(
        Some(&admission),
        UserDomainResource::BrowserTab { tab_id: "a".into() },
        false,
    )
    .unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    host.protected_request(
        "owner",
        None,
        "host.browser",
        json!({"op":"state"}),
        policy.clone(),
    )
    .unwrap();
    host.request_takeover(
        "owner",
        EnvironmentActor::new("human", EnvironmentActorKind::Human, "Human"),
        "b",
        1,
    )
    .unwrap();
    host.set_focus("owner", Some("second"));
    let scoped = host
        .protected_request_admitted(
            "owner",
            Some(&admission),
            "host.browser",
            json!({"op":"state"}),
            policy,
        )
        .unwrap();
    assert_eq!(scoped["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["tabs"][0]["tab_id"], "a");
    assert!(host.actor_snapshot("owner").unwrap()["input_ownership"]
        .to_string()
        .contains("b"));
    assert!(
        host.release_input("owner", "human", "b", 1).is_ok(),
        "MP-11: another actor's tab must remain in the authoritative registry"
    );
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mdaccess_startup_authority_rechecks_focus_without_revoking_retained_grants() {
    let host = KernelBrowserHost::new(std::env::temp_dir().join("mdaccess-startup-authority"));
    host.set_focus("owner", Some("first"));
    host.load("owner", "first").unwrap();
    let admission = host.admit("owner", "first").unwrap();
    let mut params = json!({"op":"state"});
    let focused = host
        .browser_request_authority(Some(&admission), &mut params, None)
        .unwrap();
    assert_eq!(params["_retained_agent"], false);
    assert!(!focused.requested());
    host.set_focus("owner", Some("second"));
    assert!(
        focused.requested(),
        "MP-11: startup focus guard stayed live after focus change"
    );
    assert!(
        !admission.cancellation.requested(),
        "MP-08: focus change revoked retained authority"
    );
    let retained = host
        .browser_request_authority(
            Some(&admission),
            &mut params,
            Some(admission.cancellation.clone()),
        )
        .unwrap();
    assert_eq!(params["_retained_agent"], true);
    assert!(!retained.requested());
    host.revoke_agent("first");
    assert!(
        retained.requested(),
        "MP-11: retained startup observation ignored revocation"
    );
}

#[test]
fn mdaccess_controller_startup_does_not_block_immediate_revocation() {
    let root =
        std::env::temp_dir().join(format!("mdaccess-startup-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script,r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*)
   printf 'started' > "$root/started"
   while [ ! -f "$root/release" ]; do sleep 0.01; done
   printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("owner", &script, &root);
    host.set_focus("owner", Some("first"));
    host.load("owner", "first").unwrap();
    let admission = host.admit("owner", "first").unwrap();
    let epoch = admission.cancellation.clone();
    let caller = host.clone();
    let run = std::thread::spawn(move || {
        caller.protected_request_admitted(
            "owner",
            Some(&admission),
            "host.browser",
            json!({"op":"state"}),
            json!({"values":[],"targets":[],"unknown":false}),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !root.join("started").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = root.join("started").exists();
    let revoker = host.clone();
    let revoke = std::thread::spawn(move || revoker.revoke_agent("first"));
    let deadline = Instant::now() + Duration::from_millis(500);
    while !epoch.requested() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let immediate = epoch.requested();
    // Always settle the run-owned controller before reporting a failure.
    std::fs::write(root.join("release"), "release").unwrap();
    revoke.join().unwrap();
    let outcome = run.join().unwrap();
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        started,
        "MP-10: startup fixture never reached its native handshake"
    );
    assert!(immediate, "MP-11: startup I/O blocked grant revocation");
    assert!(outcome.unwrap_err().contains("not_granted"));
}

#[test]
fn mdaccess_retained_browser_scope_allows_only_observation_and_scroll() {
    let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdaccess-scope"));
    host.set_focus("owner", Some("first"));
    host.load("owner", "first").unwrap();
    let admission = host.admit("owner", "first").unwrap();
    host.claim_resource(
        Some(&admission),
        UserDomainResource::BrowserTab {
            tab_id: "tab".into(),
        },
        false,
    )
    .unwrap();
    host.set_focus("owner", Some("second"));
    for input in [
        json!({"kind":"key","key":"Tab"}),
        json!({"kind":"key","key":"Delete"}),
        json!({"kind":"text","text":"fixture"}),
        json!({"kind":"click","x":1,"y":2}),
    ] {
        let params = json!({"op":"input","tab_id":"tab","input":input});
        assert!(host
            .scope_browser_request(Some(&admission), "host.browser", &params)
            .unwrap_err()
            .contains("sensitive_requires_focus"));
    }
    for op in ["open", "start", "stop", "navigate", "close", "unknown"] {
        assert!(host
            .scope_browser_request(
                Some(&admission),
                "host.browser",
                &json!({"op":op,"tab_id":"tab"})
            )
            .unwrap_err()
            .contains("not_focused_agent"));
    }
    for params in [
        json!({"op":"state"}),
        json!({"op":"snapshot","tab_id":"tab"}),
        json!({"op":"input","tab_id":"tab","input":{"kind":"scroll","x":1,"y":2,"delta_x":0,"delta_y":1}}),
    ] {
        host.scope_browser_request(Some(&admission), "host.browser", &params)
            .unwrap();
    }
    host.set_focus("owner", Some("first"));
    host.scope_browser_request(
        Some(&admission),
        "host.browser",
        &json!({"op":"input","tab_id":"tab","input":{"kind":"key","key":"Tab"}}),
    )
    .unwrap();
}

#[test]
fn mdaccess_retained_notes_read_but_authored_mutations_require_focus() {
    let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdaccess-note-scope"));
    host.set_focus("owner", Some("first"));
    host.load_for("owner", "first", KernelBrowserCapability::Notes)
        .unwrap();
    let admission = host
        .admit_for("owner", "first", KernelBrowserCapability::Notes)
        .unwrap();
    host.set_focus("owner", Some("second"));
    assert_eq!(
        host.note_operation("owner", Some(&admission), || Ok(42))
            .unwrap(),
        42
    );
    assert!(host
        .note_mutation::<()>("owner", Some(&admission), || panic!(
            "MP-11: retained note commit"
        ))
        .unwrap_err()
        .contains("not_focused_agent"));
    host.set_focus("owner", Some("first"));
    assert_eq!(
        host.note_mutation("owner", Some(&admission), || Ok(42))
            .unwrap(),
        42
    );
    host.revoke_agent("first");
    assert!(host
        .note_mutation::<()>("owner", Some(&admission), || panic!(
            "MP-11: revoked note commit"
        ))
        .is_err());
}
