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

/// macOS M1: a registered seat behind the same Computer adapter, without a
/// browser process. Each start is a new pairing epoch; `crash` ends the lease.
#[derive(Default)]
struct FakeSeat {
    epoch: u32,
    live: bool,
    calls: Vec<String>,
}
impl computer_backend::ComputerBackend for FakeSeat {
    fn ready(&mut self) -> bool {
        self.live
    }
    fn start(&mut self) -> Result<(), String> {
        self.calls.push("start".into());
        (self.epoch, self.live) = (self.epoch + 1, true);
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        self.calls.push("stop".into());
        self.live = false;
        Ok(())
    }
    fn request(
        &mut self,
        method: &str,
        _: Value,
        _: Option<Arc<BrowserCancellation>>,
    ) -> Result<Value, crate::error::HostFailure> {
        self.calls.push(method.into());
        if !self.live {
            return Err("fake seat stopped".into());
        }
        Ok(json!({"surface_id":"macos-seat","generation":self.epoch.to_string()}))
    }
}
fn seat_host() -> (KernelBrowserHost, Arc<Mutex<FakeSeat>>) {
    let root = std::env::temp_dir().join(format!("cumac-seat-{:032x}", rand::random::<u128>()));
    let host = KernelBrowserHost::new(root);
    let seat = Arc::new(Mutex::new(FakeSeat::default()));
    host.register_computer_seat("owner", seat.clone()).unwrap();
    (host, seat)
}
fn computer(host: &KernelBrowserHost, op: Value) -> Result<Value, String> {
    host.protected_request_admitted("owner", None, "host.computer", op, Value::Null)
        .map_err(|e| e.to_string())
}
#[test]
fn m1_registered_seat_backs_the_shared_adapter_for_one_owner() {
    let (host, seat) = seat_host();
    let other: Arc<Mutex<dyn computer_backend::ComputerBackend>> =
        Arc::new(Mutex::new(FakeSeat::default()));
    assert!(host.register_computer_seat("intruder", other).is_err());
    assert!(computer(&host, json!({"op":"state"})).is_err());
    let state = computer(&host, json!({"op":"start"})).unwrap();
    assert_eq!(state["generation"], "1");
    assert_eq!(
        seat.lock().unwrap().calls,
        ["start", "host.protect", "host.computer"]
    );
    // No browser process was created for the native seat.
    assert!(host.inner.lock().unwrap().browsers.is_empty());
}
#[test]
fn m1_agent_cannot_start_a_personal_seat() {
    let (host, seat) = seat_host();
    host.set_focus("owner", Some("a"));
    host.load_for("owner", "a", KernelBrowserCapability::Computer)
        .unwrap();
    let admission = host
        .admit_for("owner", "a", KernelBrowserCapability::Computer)
        .unwrap();
    assert!(host
        .protected_request_admitted(
            "owner",
            Some(&admission),
            "host.computer",
            json!({"op":"start"}),
            Value::Null
        )
        .is_err());
    assert!(seat.lock().unwrap().calls.is_empty());
}
#[test]
fn m1_crash_requires_explicit_restart_and_old_epoch_is_stale() {
    let (host, seat) = seat_host();
    let old = computer(&host, json!({"op":"start"})).unwrap();
    seat.lock().unwrap().live = false;
    // Observation never restarts a crashed helper.
    assert!(computer(&host, json!({"op":"state"})).is_err());
    let fresh = computer(&host, json!({"op":"start"})).unwrap();
    assert_ne!(fresh["generation"], old["generation"]);
    let mut stale = json!({"op":"snapshot"});
    stale["surface_id"] = old["surface_id"].clone();
    stale["generation"] = old["generation"].clone();
    assert!(computer(&host, stale)
        .unwrap_err()
        .contains("stale native desktop"));
}
#[test]
fn m1_revoke_all_and_shutdown_stop_the_seat_but_one_agent_does_not() {
    let (host, seat) = seat_host();
    computer(&host, json!({"op":"start"})).unwrap();
    host.revoke_grants("owner", Some("a"));
    std::thread::sleep(Duration::from_millis(100));
    assert!(seat.lock().unwrap().live);
    host.revoke_grants("owner", None);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while seat.lock().unwrap().live && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!seat.lock().unwrap().live);
    assert!(computer(&host, json!({"op":"state"})).is_err());
    computer(&host, json!({"op":"start"})).unwrap();
    host.shutdown().unwrap();
    assert!(!seat.lock().unwrap().live);
}
