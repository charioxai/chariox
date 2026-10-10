//! MP-08 / MP-11: fail-first desktop grants and Browser/Computer arbitration.
use super::*;
use serde_json::json;
#[test]
fn mp11_desktop_grant_is_explicit_retained_scoped_and_epoch_fenced() {
    let root = std::env::temp_dir().join(format!("culinux-grants-{:032x}", rand::random::<u128>()));
    let host = KernelBrowserHost::new(root);
    host.set_focus("alice", Some("a"));
    host.load_for("alice", "a", KernelBrowserCapability::Computer)
        .unwrap();
    let admission = host
        .admit_for("alice", "a", KernelBrowserCapability::Computer)
        .unwrap();
    let ended_run = admission.clone().with_authority(|| false);
    assert!(host.check_admission(Some(&ended_run)).is_err());
    let desktop = UserDomainResource::Desktop {
        surface_id: "owned".into(),
    };
    host.claim_resource(Some(&admission), desktop.clone(), false)
        .unwrap();
    host.set_focus("alice", Some("b"));
    host.claim_resource(Some(&admission), desktop, false)
        .unwrap();
    assert!(host
        .claim_resource(
            Some(&admission),
            UserDomainResource::Desktop {
                surface_id: "other".into()
            },
            false
        )
        .is_err());
    assert!(host
        .admit_for("bob", "a", KernelBrowserCapability::Computer)
        .is_err());
    host.revoke_grants("alice", Some("a"));
    assert!(host.check_admission(Some(&admission)).is_err());
    host.set_focus("alice", Some("a"));
    host.load_for("alice", "a", KernelBrowserCapability::Computer)
        .unwrap();
    assert!(host.check_admission(Some(&admission)).is_err());
}
#[test]
fn mp11_human_desktop_takeover_cancels_browser_input_and_fences_native_input() {
    let mut model = KernelBrowserActors::default();
    model
        .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"t","document_id":"d"}]}))
        .unwrap();
    model
        .reconcile_desktop(&json!({"surface_id":"s","generation":"g"}))
        .unwrap();
    let agent = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "A");
    let human = EnvironmentActor::new("terminal:a", EnvironmentActorKind::Human, "Human");
    let (action, cancel) = model
        .begin(
            agent.clone(),
            &json!({"op":"input","generation":1,"tab_id":"t"}),
        )
        .unwrap();
    assert!(model
        .takeover_desktop(human.clone(), "foreign", "g")
        .is_err());
    assert!(matches!(
        model.takeover_desktop(human, "s", "g").unwrap(),
        TakeoverOutcome::CancellationRequired { .. }
    ));
    assert!(cancel.requested());
    assert!(model
        .begin(agent.clone(), &json!({"op":"native_input","generation":1}))
        .is_err());
    model.finish(&action, EnvironmentActionTerminal::Cancelled);
    assert!(model
        .begin(
            agent.clone(),
            &json!({"op":"input","generation":1,"tab_id":"t"})
        )
        .is_err());
    assert!(model.release_desktop("terminal:b", "s", "g").is_err());
    assert!(model
        .check_desktop_release("terminal:a", "s", "old")
        .is_err());
    assert!(model.check_desktop_release("terminal:a", "s", "g").is_ok());
    model.release_desktop("terminal:a", "s", "g").unwrap();
    assert!(model
        .begin(agent, &json!({"op":"native_input","generation":1}))
        .is_ok());
}

#[test]
fn mp11_browser_tab_takeover_also_fences_and_cancels_whole_desktop_input() {
    let mut model = KernelBrowserActors::default();
    model
        .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"t","document_id":"d"}]}))
        .unwrap();
    model
        .reconcile_desktop(&json!({"surface_id":"s","generation":"g"}))
        .unwrap();
    let agent = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Agent");
    let human = EnvironmentActor::new("terminal:a", EnvironmentActorKind::Human, "Human");
    let input = json!({"op":"input","_native":true});
    model.takeover(human.clone(), "t", 1).unwrap();
    assert!(model.begin(agent.clone(), &input).is_err());
    model.release(&human.actor_id, "t", 1).unwrap();
    let (id, cancel) = model.begin(agent.clone(), &input).unwrap();
    model.takeover(human, "t", 1).unwrap();
    assert!(cancel.requested());
    model.finish(&id, EnvironmentActionTerminal::Cancelled);
    assert!(model.begin(agent, &input).is_err());
}

#[test]
fn mp08_mp10_mp11_warm_human_input_keeps_live_desktop_without_browser_reconcile() {
    let root = std::env::temp_dir().join(format!(
        "culinux-warm-admission-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) result='{"state":"ready","process_id":'$$'}' ;;
 *'"method":"host.protect"'*) printf 'protect\n' >> "$root/protection"; result='{}' ;;
 *'"method":"host.browser"'*) printf 'browser\n' >> "$root/browser"; result='{"generation":1,"tabs":[]}' ;;
 *'"op":"state"'*|*'"op":"start"'*)
   printf 'desktop\n' >> "$root/desktop"
   generation=g; if test -f "$root/stale"; then generation=g2; fi
   result='{"surface_id":"s","generation":"'"$generation"'"}' ;;
 *'"op":"input"'*) printf 'input\n' >> "$root/input"; result='{"applied":true}' ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null}}\n' "$id"; exit 0 ;;
 *) result='{}' ;;
 esac
 printf '{"id":%s,"ok":true,"result":%s}\n' "$id" "$result"
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
    let lifetime = crate::runtime::command::TerminalLifetime::default();
    let human = host.admit_terminal("alice", lifetime.clone());
    let policy = json!({"values":[],"targets":[],"unknown":false});
    let send = |admission: &KernelBrowserAdmission, request| {
        host.protected_request_admitted(
            "alice",
            Some(admission),
            "host.computer",
            request,
            policy.clone(),
        )
    };
    send(&human, json!({"op":"start"})).unwrap();
    std::fs::write(root.join("browser"), b"").unwrap();
    let input = json!({"op":"input","surface_id":"s","generation":"g","input":{"kind":"text","text":"public"}});
    assert_eq!(send(&human, input.clone()).unwrap()["applied"], true);
    let human_browser = std::fs::read(root.join("browser")).unwrap();
    assert!(!std::fs::read(root.join("desktop")).unwrap().is_empty());
    send(
        &human,
        json!({"op":"release","surface_id":"s","generation":"g"}),
    )
    .unwrap();
    host.set_focus("alice", Some("agent"));
    host.load_for("alice", "agent", KernelBrowserCapability::Computer)
        .unwrap();
    let agent = host
        .admit_for("alice", "agent", KernelBrowserCapability::Computer)
        .unwrap();
    send(&agent, json!({"op":"start"})).unwrap();
    std::fs::write(root.join("browser"), b"").unwrap();
    assert_eq!(send(&agent, input.clone()).unwrap()["applied"], true);
    let agent_browser = std::fs::read(root.join("browser")).unwrap();
    let dispatched = std::fs::read(root.join("input")).unwrap();
    std::fs::write(root.join("stale"), b"changed generation").unwrap();
    assert!(send(&human, input.clone()).is_err());
    assert_eq!(
        std::fs::read(root.join("input")).unwrap(),
        dispatched,
        "stale native generation emits no input"
    );
    lifetime.cancel();
    assert!(send(&human, input).is_err());
    assert_eq!(
        std::fs::read(root.join("input")).unwrap(),
        dispatched,
        "retired terminal emits no input"
    );
    host.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        human_browser.is_empty(),
        "warm human input waited for unrelated Browser reconciliation"
    );
    assert!(
        !agent_browser.is_empty(),
        "agent input must retain fresh Browser reconciliation"
    );
}
