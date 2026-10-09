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
// MP-08/MP-11 (coordinator option b, 2026-10-09): Desktop ownership of a
// human actor lasts while one of its desktop video subscriptions is live.
fn desktop_model() -> KernelBrowserActors {
    let mut model = KernelBrowserActors::default();
    model
        .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"t","document_id":"d"}]}))
        .unwrap();
    model
        .reconcile_desktop(&json!({"surface_id":"s","generation":"g"}))
        .unwrap();
    model
}
#[test]
fn mp11_disconnect_without_release_retires_desktop_ownership_at_viewer_lease_expiry() {
    use crate::runtime::kernel_browser_actors::DESKTOP_VIEWER_LEASE;
    let mut model = desktop_model();
    let old = EnvironmentActor::new("terminal:old", EnvironmentActorKind::Human, "Human");
    let fresh = EnvironmentActor::new("terminal:new", EnvironmentActorKind::Human, "Human");
    let input = json!({"op":"input","generation":1,"tab_id":"t"});
    let start = std::time::Instant::now();
    model.desktop_viewer("terminal:old", "view-old", start);
    model.takeover_desktop(old, "s", "g").unwrap();
    // The old web client vanished without a release (no relay gone event).
    // While its lease lives, the fresh client's Browser input is refused.
    assert!(model.begin(fresh.clone(), &input).is_err());
    model.renew_desktop_viewer("view-old", start + DESKTOP_VIEWER_LEASE / 2);
    model.retire_lapsed_desktop_viewers(start + DESKTOP_VIEWER_LEASE);
    assert!(
        model.desktop_owner_is("terminal:old"),
        "a renewed lease keeps ownership"
    );
    model.retire_lapsed_desktop_viewers(start + DESKTOP_VIEWER_LEASE * 3 / 2);
    assert!(
        !model.desktop_owner_is("terminal:old"),
        "the lapsed lease retires ownership"
    );
    let (action, _) = model.begin(fresh.clone(), &input).unwrap();
    model.finish(&action, EnvironmentActionTerminal::Completed);
    model.takeover_desktop(fresh, "s", "g").unwrap();
    assert!(model.desktop_owner_is("terminal:new"));
}
#[test]
fn mp11_fast_close_reopen_and_second_viewer_keep_desktop_ownership_correct() {
    let mut model = desktop_model();
    let old = EnvironmentActor::new("terminal:old", EnvironmentActorKind::Human, "Human");
    let fresh = EnvironmentActor::new("terminal:new", EnvironmentActorKind::Human, "Human");
    let now = std::time::Instant::now();
    model.desktop_viewer("terminal:old", "panel", now);
    model.desktop_viewer("terminal:old", "popout", now);
    model.takeover_desktop(old.clone(), "s", "g").unwrap();
    // One of two viewers closes: the other keeps the actor's ownership.
    model.end_desktop_viewer("panel", now);
    model.retire_lapsed_desktop_viewers(now);
    assert!(model.desktop_owner_is("terminal:old"));
    // Orderly close: the acknowledged release frees the desktop at once.
    model.release_desktop("terminal:old", "s", "g").unwrap();
    model.end_desktop_viewer("popout", now);
    model.desktop_viewer("terminal:new", "reopened", now);
    model.takeover_desktop(fresh, "s", "g").unwrap();
    assert!(model.desktop_owner_is("terminal:new"));
    // An unsubscribe without release (older viewer) retires on the next check.
    let input = json!({"op":"input","generation":1,"tab_id":"t"});
    assert!(
        model.begin(old.clone(), &input).is_err(),
        "the reopened owner holds the desktop"
    );
    model.end_desktop_viewer("reopened", now);
    let (action, _) = model.begin(old, &input).unwrap();
    model.finish(&action, EnvironmentActionTerminal::Completed);
    assert!(!model.desktop_owner_is("terminal:new"));
}

#[test]
fn mp10_desktop_display_wait_does_not_block_input_or_cross_responses() {
    use std::time::Instant;
    let root = std::env::temp_dir().join(format!(
        "culinux-desktop-wait-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    std::fs::write(&script, r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf 'health\n' >> "$root/health"; result='{"state":"ready","process_id":'$$'}' ;;
 *'"method":"host.protect"'*) printf 'policy\n' >> "$root/policy"; result='{}' ;;
 *'"method":"host.browser"'*) result='{"generation":1,"tabs":[]}' ;;
 *'"op":"state"'*) result='{"surface_id":"s","generation":"g"}' ;;
 *'"op":"screenshot"'*)
   printf 'started\n' > "$root/capture"
   (while ! test -f "$root/release"; do sleep 0.01; done; printf '{"id":%s,"ok":true,"result":{"frame_sent":false}}\n' "$id") &
   continue ;;
 *'"op":"input"'*) result='{"input_completed":true}' ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null}}\n' "$id"; exit 0 ;;
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
    let startup_health = std::fs::read(root.join("health")).unwrap();
    let policy = json!({"values":[],"targets":[],"unknown":false});
    let (tx, rx) = std::sync::mpsc::channel();
    let early = std::thread::scope(|scope| {
        let capture=scope.spawn(|| host.protected_request("alice",None,"host.computer",json!({"op":"screenshot","surface_id":"s","generation":"g","display_subscription_id":"view"}),policy.clone()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.join("capture").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("capture").exists());
        let input=scope.spawn(|| {tx.send(host.protected_request("alice",None,"host.computer",json!({"op":"input","surface_id":"s","generation":"g","observed_by":"terminal:human","input":{"kind":"key","key":"PageDown"}}),policy.clone())).unwrap();});
        let early = rx.recv_timeout(Duration::from_millis(100));
        std::fs::write(root.join("release"), b"release").unwrap();
        assert_eq!(capture.join().unwrap().unwrap()["frame_sent"], false);
        input.join().unwrap();
        early
    });
    host.shutdown().unwrap();
    assert_eq!(std::fs::read(root.join("health")).unwrap(), startup_health);
    assert_eq!(std::fs::read(root.join("policy")).unwrap(), b"policy\n");
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        early
            .expect("MP-10: input waited behind desktop frame")
            .unwrap()["input_completed"],
        true
    );
}
