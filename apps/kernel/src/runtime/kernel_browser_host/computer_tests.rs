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
