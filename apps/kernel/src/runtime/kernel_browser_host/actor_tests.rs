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
fn mdaccess_focus_switch_preserves_running_input() {
    check_running_cancellation(false, true);
}
fn check_running_cancellation(revoke: bool, focus_change: bool) {
    let root = std::env::temp_dir().join(format!(
        "chariox-md3-takeover-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    if focus_change {
        std::fs::write(root.join("focus-retention"), "owned").unwrap();
    }
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
    *'"op":"input"'*) pending=$id; printf 'started\n' > "$root/started"
      if [ -f "$root/focus-retention" ]; then
        while [ ! -f "$root/release" ]; do sleep 0.01; done
        printf '{"id":%s,"ok":true,"result":{}}\n' "$id"
      fi ;;
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
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    let mutation = json!({"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"d","input":{"kind":"text","text":"must-not-be-retained"}});
    let policy = json!({"values":[],"targets":[],"unknown":false});
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            let admission = host.admit("alice", "agent").unwrap();
            host.protected_request_admitted(
                "alice",
                Some(&admission),
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
            assert!(host.has_grant("alice", "agent"));
            assert!(!host
                .admit("alice", "agent")
                .unwrap()
                .cancellation
                .requested());
            std::fs::write(root.join("release"), "release").unwrap();
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
        let result = running.join().unwrap();
        if focus_change {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            if revoke {
                assert!(matches!(
                    error,
                    crate::error::HostFailure::Refused(
                        crate::error::UserDomainRefusalReason::NotGranted
                    )
                ));
            } else {
                assert!(
                    matches!(error, crate::error::HostFailure::Other(message) if message.contains("browser_action_cancelled"))
                );
            }
        }
    });
    assert_eq!(root.join("cancelled").exists(), !focus_change);
    if revoke {
        assert!(!host.has_grant("alice", "agent"));
        assert!(host.actor_snapshot("alice").unwrap()["actors"]
            .as_array()
            .unwrap()
            .is_empty());
    } else if !focus_change {
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
    assert!(
        !result.unwrap(),
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
    host.backend("owner")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
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
fn mdaccess_focus_change_keeps_inflight_grant_authority_until_revoke() {
    let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdaccess-authority"));
    host.set_focus("owner", Some("first"));
    host.load("owner", "first").unwrap();
    let admission = host.admit("owner", "first").unwrap();
    host.set_focus("owner", Some("second"));
    assert!(!admission.cancellation.requested());
    host.check_admission(Some(&admission)).unwrap();
    host.revoke_agent("first");
    assert!(admission.cancellation.requested());
    assert!(host
        .check_admission(Some(&admission))
        .unwrap_err()
        .contains("not_granted"));
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
            json!({"op":"start"}),
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
    assert!(matches!(
        outcome.unwrap_err(),
        crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
    ));
}

#[test]
fn mdaccess_retained_browser_scope_allows_input_but_no_new_resources_or_vault() {
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
        host.scope_browser_request(
            Some(&admission),
            "host.browser",
            &json!({"op":"input","tab_id":"tab","input":input}),
        )
        .unwrap();
    }
    for op in ["state", "start", "navigate", "close"] {
        host.scope_browser_request(
            Some(&admission),
            "host.browser",
            &json!({"op":op,"tab_id":"tab"}),
        )
        .unwrap();
    }
    // MP-08/MP-11 SB-02: Stop is global even with a scoped tab parameter.
    assert!(host
        .scope_browser_request(
            Some(&admission),
            "host.browser",
            &json!({"op":"stop","tab_id":"tab"})
        )
        .unwrap_err()
        .contains("not_focused_agent"));
    assert!(host
        .scope_browser_request(
            Some(&admission),
            "host.browser",
            &json!({"op":"snapshot","tab_id":"new"})
        )
        .unwrap_err()
        .contains("not_focused_agent"));
    assert!(host
        .scope_browser_request(Some(&admission), "host.secret", &json!({"tab_id":"tab"}))
        .unwrap_err()
        .contains("sensitive_requires_focus"));
    host.revoke_agent("first");
    assert!(host
        .check_admission(Some(&admission))
        .unwrap_err()
        .contains("not_granted"));
}

#[test]
fn mdaccess_retained_notes_commits_share_grant_and_revoke_boundary() {
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
    host.revoke_agent("first");
    assert!(host
        .note_operation::<()>("owner", Some(&admission), || panic!(
            "MP-11: revoked note commit"
        ))
        .is_err());
}

#[test]
fn mp11_global_stop_requires_focus_even_with_retained_tabs() {
    for owns_tab in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "chariox-mp11-stop-scope-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("controller.sh");
        // A real stdio controller lifetime. Browser resources are fixture IDs;
        // this tests kernel admission/shutdown, not Chromium/client acceptance.
        std::fs::write(
            &script,
            r#"set -eu
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) result=$(printf '{"state":"ready","process_id":%s}' "$$") ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
    *) result='{}' ;;
  esac
  printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$result"
done
"#,
        )
        .unwrap();
        let host = KernelBrowserHost::new(root.clone());
        let backend = BrowserControllerProcessStdioBackend::new(
            "/bin/sh",
            vec![script.display().to_string()],
            Duration::from_secs(2),
        )
        .for_host();
        host.inner
            .lock()
            .unwrap()
            .browsers
            .insert("owner".into(), Arc::new(Mutex::new(backend)));
        host.backend("owner")
            .unwrap()
            .lock()
            .unwrap()
            .start()
            .unwrap();
        host.set_focus("owner", Some("first"));
        host.load("owner", "first").unwrap();
        let first = host.admit("owner", "first").unwrap();
        if owns_tab {
            host.claim_resource(
                Some(&first),
                UserDomainResource::BrowserTab {
                    tab_id: "first-tab".into(),
                },
                false,
            )
            .unwrap();
        }
        host.set_focus("owner", Some("second"));
        host.load("owner", "second").unwrap();
        let second = host.admit("owner", "second").unwrap();
        host.claim_resource(
            Some(&second),
            UserDomainResource::BrowserTab {
                tab_id: "second-tab".into(),
            },
            false,
        )
        .unwrap();
        let mut refusals = Vec::new();
        // Exercise the actual tabless Stop path, and reject tab_id smuggling.
        for params in [
            json!({"op":"stop"}),
            json!({"op":"stop","tab_id":"first-tab"}),
        ] {
            refusals.push(host.protected_request_admitted(
                "owner",
                Some(&first),
                "host.browser",
                params,
                json!({"values":[],"targets":[],"unknown":false}),
            ));
        }
        let still_running = host
            .backend("owner")
            .unwrap()
            .lock()
            .unwrap()
            .health()
            .unwrap()
            .state
            == BrowserControllerProcessState::Ready;
        let second_can_stop =
            host.scope_browser_request(Some(&second), "host.browser", &json!({"op":"stop"}));
        let owner_can_stop =
            host.scope_browser_request(None, "host.browser", &json!({"op":"stop"}));
        let first_retained = host.has_grant("owner", "first");
        host.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        for result in refusals {
            assert!(
                matches!(
                    result,
                    Err(crate::error::HostFailure::Refused(
                        crate::error::UserDomainRefusalReason::NotFocusedAgent
                    ))
                ),
                "MP-11: retained global Stop was admitted: {result:?}"
            );
        }
        assert!(
            still_running,
            "MP-11: retained Stop shut down the shared controller"
        );
        second_can_stop.unwrap();
        owner_can_stop.unwrap();
        assert!(first_retained);
    }
}

#[test]
fn mdaccess_idle_lapse_refuses_retained_keys_text_clicks_and_note_commits() {
    let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdaccess-idle-input"));
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
    let now = Instant::now();
    host.inner.lock().unwrap().access.bind(
        "owner",
        "first",
        "session",
        true,
        (now, 0),
        Duration::from_secs(1),
    );
    host.inner.lock().unwrap().access.bind(
        "owner",
        "first",
        "session",
        false,
        (now, 0),
        Duration::from_secs(1),
    );
    assert!(host.inner.lock().unwrap().access.bind(
        "owner",
        "first",
        "session",
        false,
        (now + Duration::from_secs(1), 1000),
        Duration::from_secs(1)
    ));
    for input in [
        json!({"kind":"key","key":"Delete"}),
        json!({"kind":"text","text":"expired"}),
        json!({"kind":"click","x":1,"y":2}),
    ] {
        let error = host
            .protected_request_admitted(
                "owner",
                Some(&admission),
                "host.browser",
                json!({"op":"input","tab_id":"tab","input":input}),
                Value::Null,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
        ));
    }
    assert!(host
        .note_operation::<()>("owner", Some(&admission), || panic!(
            "MP-11: expired commit"
        ))
        .unwrap_err()
        .contains("not_granted"));
}

// MP-08/MP-11: deterministic post-controller/pre-registration revoke/refocus race.
#[test]
fn mdaccess_revoked_open_cannot_populate_refocused_grant() {
    check_result_epoch_race("open", true);
}
#[test]
fn mdaccess_revoked_subscribe_cannot_populate_refocused_grant() {
    check_result_epoch_race("subscribe", true);
}
#[test]
fn mdaccess_revoked_snapshot_cannot_return_after_refocus() {
    check_result_epoch_race("snapshot", true);
}
#[test]
fn mdaccess_result_rechecks_provider_run_authority_before_return() {
    check_result_epoch_race("snapshot", false);
}
fn check_result_epoch_race(op: &str, refocus: bool) {
    let root = std::env::temp_dir().join(format!(
        "chariox-mdaccess-result-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) result=$(printf '{"state":"ready","process_id":%s}' "$$") ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
    *'"op":"open"'*) result='{"generation":1,"tab_id":"late-tab","tabs":[{"tab_id":"late-tab","document_id":"d"}]}' ;;
    *'"op":"subscribe"'*) result='{"generation":1,"subscription_id":"late-sub"}' ;;
    *) result='{"generation":1,"tabs":[{"tab_id":"existing-tab","document_id":"d"}]}' ;;
  esac
  printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$result"
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    let backend = BrowserControllerProcessStdioBackend::new(
        "/bin/sh",
        vec![script.display().to_string()],
        Duration::from_secs(2),
    )
    .for_host();
    host.inner
        .lock()
        .unwrap()
        .browsers
        .insert("owner".into(), Arc::new(Mutex::new(backend)));
    host.set_focus("owner", Some("agent"));
    host.load("owner", "agent").unwrap();
    host.backend("owner")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let run = live.clone();
    let admission = host
        .admit("owner", "agent")
        .unwrap()
        .with_authority(move || run.load(std::sync::atomic::Ordering::Acquire));
    let caller = host.clone();
    host.inner.lock().unwrap().after_controller_check = Some(Arc::new(move || {
        if refocus {
            caller.revoke_agent("agent");
            caller.set_focus("owner", Some("agent"));
            caller.load("owner", "agent").unwrap();
        } else {
            live.store(false, std::sync::atomic::Ordering::Release);
        }
    }));
    let mut params = json!({"op":op});
    if op != "open" {
        params["tab_id"] = "existing-tab".into();
    }
    let result = host.protected_request_admitted(
        "owner",
        Some(&admission),
        "host.browser",
        params,
        json!({"values":[],"targets":[],"unknown":false}),
    );
    let state = host.inner.lock().unwrap();
    let grant = state.access.grant("owner", "agent").unwrap();
    let uncontaminated = !grant.resources.contains(&UserDomainResource::BrowserTab {
        tab_id: "late-tab".into(),
    }) && grant.subscriptions.is_empty();
    drop(state);
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        uncontaminated,
        "MP-11: revoked result populated the fresh grant"
    );
    assert!(
        matches!(
            result.unwrap_err(),
            crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
        ),
        "MP-11: old request crossed result authority boundary"
    );
}

#[test]
fn authority_callback_does_not_hold_actor_lock_during_concurrent_retirement() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for retire in [false, true] {
        for op in ["input", "stop", "takeover"] {
            let root = crate::test_support::TestWorktree::new("md-authority-lock-order");
            let script = root.path().join("controller.sh");
            std::fs::write(&script, r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}}\n' "$id" ;;
  *'"op":"input"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
            let host = KernelBrowserHost::new(root.path().to_path_buf());
            host.install_fixture_backend("alice", &script, root.path());
            host.set_focus("alice", Some("agent"));
            host.load("alice", "agent").unwrap();
            let model = host.actor_model("alice").unwrap();
            model
                .lock()
                .unwrap()
                .reconcile(
                    &json!({"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}),
                )
                .unwrap();
            let callback_under_lock = Arc::new(AtomicBool::new(false));
            let inspect = Arc::new(AtomicBool::new(true));
            let detected = callback_under_lock.clone();
            let inspecting = inspect.clone();
            let authority_host = host.clone();
            let authority_model = model.clone();
            let admission = host
                .admit("alice", "agent")
                .unwrap()
                .with_authority(move || {
                    // Detect the old inversion without leaving blocked test threads.
                    // Before racing retirement no other thread can own this mutex.
                    if inspecting.load(Ordering::Acquire) && authority_model.try_lock().is_err() {
                        detected.store(true, Ordering::Release);
                        return false;
                    }
                    // Production authority reaches this host.inner acquisition.
                    authority_host.is_focused("alice", "agent")
                });
            let call = || {
                if op == "takeover" {
                    host.request_takeover_admitted(
                        "alice",
                        EnvironmentActor::new("terminal:a", EnvironmentActorKind::Human, "Alice"),
                        "host-tab-a",
                        1,
                        Some(&admission),
                    )
                    .map(|_| ())
                    .map_err(crate::error::HostFailure::from)
                } else {
                    host.protected_request_admitted("alice", Some(&admission), "host.browser", json!({"op":op,"tab_id":"host-tab-a","generation":1,"document_id":"d","input":{"kind":"text","text":"fixture"}}), json!({"values":[],"targets":[],"unknown":false})).map(|_| ())
                }
            };
            let initial = call();
            assert!(
                !callback_under_lock.load(Ordering::Acquire),
                "{op}: authority callback ran under the actor-model mutex"
            );
            initial.unwrap();
            inspect.store(false, Ordering::Release);
            std::thread::scope(|scope| {
                let running = scope.spawn(call);
                let retirement = scope.spawn(|| {
                    if retire {
                        host.revoke_agent("agent");
                    } else {
                        host.set_focus("alice", None);
                    }
                });
                let _ = running.join().unwrap();
                retirement.join().unwrap();
            });
            assert!(host.check_admission(Some(&admission)).is_err());
            host.shutdown().unwrap();
        }
    }
}

#[test]
fn md_display_held_capture_does_not_hold_input_ownership_or_steal_its_response() {
    let root = std::env::temp_dir().join(format!(
        "chariox-md-display-unlocked-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script,r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}}\n' "$id" ;;
 *'"op":"screenshot"'*)
   printf 'started\n' > "$root/capture"
   (while ! test -f "$root/release"; do sleep 0.01; done; printf '{"id":%s,"ok":true,"result":{"generation":1,"frame_sent":false,"display_frame":null}}\n' "$id") & ;;
 *'"op":"input"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"input_completed":true}}\n' "$id" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null}}\n' "$id";exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("alice", &script, &root);
    // MP-11: observations cannot implicitly start a stopped controller.
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    let (tx, rx) = std::sync::mpsc::channel();
    let early = std::thread::scope(|scope| {
        let capture = scope.spawn(|| {
            host.protected_request(
                "alice",
                None,
                "host.browser",
                json!({"op":"screenshot","display_subscription_id":"s"}),
                policy.clone(),
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !root.join("capture").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("capture").exists());
        let input=scope.spawn(|| { let result=host.protected_request("alice",None,"host.browser",json!({"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"d","input":{"kind":"click","x":1,"y":1}}),policy.clone());tx.send(result).unwrap();});
        let early = rx.recv_timeout(Duration::from_millis(100));
        std::fs::write(root.join("release"), b"release").unwrap();
        assert_eq!(capture.join().unwrap().unwrap()["frame_sent"], false);
        input.join().unwrap();
        early
    });
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        early.expect("input waited behind display work").unwrap()["input_completed"],
        true
    );
}

#[test]
fn mp08_display_held_input_does_not_starve_capture_or_steal_its_response() {
    let root = std::env::temp_dir().join(format!(
        "chariox-mp08-input-unlocked-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script,r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
 *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}]}}\n' "$id" ;;
 *'"op":"input"'*)
   printf 'started\n' > "$root/capture"
   (while ! test -f "$root/release"; do sleep 0.01; done; printf '{"id":%s,"ok":true,"result":{"generation":1,"input_completed":true}}\n' "$id") & ;;
 *'"op":"screenshot"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"frame_sent":false,"display_frame":null}}\n' "$id" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null}}\n' "$id";exit 0 ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("alice", &script, &root);
    // MP-11: observations cannot implicitly start a stopped controller.
    host.backend("alice")
        .unwrap()
        .lock()
        .unwrap()
        .start()
        .unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    let (tx, rx) = std::sync::mpsc::channel();
    let early = std::thread::scope(|scope| {
        let capture = scope.spawn(|| {
            host.protected_request(
                "alice",
                None,
                "host.browser",
                json!({"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"d","input":{"kind":"scroll","x":1,"y":1,"delta_x":0,"delta_y":120}}),
                policy.clone(),
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !root.join("capture").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("capture").exists());
        let input = scope.spawn(|| {
            let result = host.protected_request(
                "alice",
                None,
                "host.browser",
                json!({"op":"screenshot","display_subscription_id":"s"}),
                policy.clone(),
            );
            tx.send(result).unwrap();
        });
        let early = rx.recv_timeout(Duration::from_millis(100));
        std::fs::write(root.join("release"), b"release").unwrap();
        assert_eq!(capture.join().unwrap().unwrap()["input_completed"], true);
        input.join().unwrap();
        early
    });
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        early.expect("capture waited behind input work").unwrap()["frame_sent"],
        false
    );
}

#[test]
fn mirror_reads_and_cleanup_never_start_a_stopped_controller() {
    let root = std::env::temp_dir().join(format!(
        "chariox-mirror-no-start-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
printf 'started' > "$1/started"
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  *) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 esac
done
"#).unwrap();
    let host = KernelBrowserHost::new(root.clone());
    host.install_fixture_backend("owner", &script, &root);
    let outcomes: Vec<_> = ["mirror_subscribe", "mirror_next", "mirror_close"]
        .into_iter()
        .map(|op| {
            let params = json!({"op":op,"tab_id":"a","subscription_id":"s","generation":1});
            let outcome = host.protected_request(
                "owner",
                None,
                "host.browser",
                params,
                json!({"values":[],"targets":[],"unknown":false}),
            );
            (op, outcome, root.join("started").exists())
        })
        .collect();
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    for (op, outcome, started) in outcomes {
        assert!(!started, "MP-11: {op} started the browser controller");
        assert!(
            outcome.unwrap_err().contains("browser_unavailable"),
            "MP-11: {op} must refuse while stopped"
        );
    }
}
