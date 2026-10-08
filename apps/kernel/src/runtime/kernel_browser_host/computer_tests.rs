//! MP-08 / MP-11: fail-first desktop grants and Browser/Computer arbitration.
use super::*;
use crate::session::DEFAULT_LOCAL_USER_ID as OWNER;
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
    host.register_computer_seat(OWNER, seat.clone()).unwrap();
    (host, seat)
}
fn computer(host: &KernelBrowserHost, op: Value) -> Result<Value, String> {
    host.protected_request_admitted(OWNER, None, "host.computer", op, Value::Null)
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
    host.set_focus(OWNER, Some("a"));
    host.load_for(OWNER, "a", KernelBrowserCapability::Computer)
        .unwrap();
    let admission = host
        .admit_for(OWNER, "a", KernelBrowserCapability::Computer)
        .unwrap();
    assert!(host
        .protected_request_admitted(
            OWNER,
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
    host.revoke_grants(OWNER, Some("a"));
    assert!(seat.lock().unwrap().live);
    host.revoke_grants(OWNER, None);
    assert!(!seat.lock().unwrap().live);
    assert!(computer(&host, json!({"op":"state"})).is_err());
    computer(&host, json!({"op":"start"})).unwrap();
    host.shutdown().unwrap();
    assert!(!seat.lock().unwrap().live);
}
#[test]
fn m1_non_owner_first_cannot_take_the_seat() {
    let host = KernelBrowserHost::new(std::env::temp_dir().join("cumac-ff"));
    let seat: Arc<Mutex<dyn computer_backend::ComputerBackend>> =
        Arc::new(Mutex::new(FakeSeat::default()));
    assert!(host.register_computer_seat("intruder", seat).is_err());
}
#[test]
fn m1_restart_right_after_revoke_all_gets_a_fresh_epoch() {
    let (host, _) = seat_host();
    for _ in 0..20 {
        let old = computer(&host, json!({"op":"start"})).unwrap();
        host.revoke_grants(OWNER, None);
        let fresh = computer(&host, json!({"op":"start"})).unwrap();
        assert_ne!(fresh["generation"], old["generation"]);
    }
}
#[test]
fn m1_second_first_use_keeps_the_started_seat() {
    let (host, seat) = seat_host();
    computer(&host, json!({"op":"start"})).unwrap();
    let other: Arc<Mutex<dyn computer_backend::ComputerBackend>> =
        Arc::new(Mutex::new(FakeSeat::default()));
    host.register_computer_seat(OWNER, other).unwrap();
    let first: Arc<Mutex<dyn computer_backend::ComputerBackend>> = seat;
    assert!(Arc::ptr_eq(&host.computer_backend(OWNER).unwrap(), &first));
}

#[cfg(target_os = "macos")]
#[test]
fn m1_non_owner_backend_and_actor_requests_are_refused_before_configuration() {
    let (host, seat) = seat_host();
    assert!(host
        .computer_backend("intruder")
        .err()
        .unwrap()
        .contains("kernel authority owner"));
    for op in ["start", "state", "actors", "takeover", "release", "input"] {
        let error = host
            .protected_request_admitted(
                "intruder",
                None,
                "host.computer",
                json!({"op":op}),
                Value::Null,
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("kernel authority owner"),
            "{op}: {error}"
        );
    }
    assert!(seat.lock().unwrap().calls.is_empty());
    assert!(!host.inner.lock().unwrap().actors.contains_key("intruder"));
}

#[test]
fn m1_owner_admission_precedes_the_factory_and_concurrent_first_use_creates_once() {
    let host = KernelBrowserHost::new(std::env::temp_dir().join("cumac-owner-factory"));
    assert!(host
        .computer_seat_or_create("intruder", || panic!(
            "foreign user reached helper configuration"
        ))
        .is_err());
    let created = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let (host, created, barrier) = (host.clone(), created.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                host.computer_seat_or_create(OWNER, || {
                    created.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(Some(Arc::new(Mutex::new(FakeSeat::default()))))
                })
                .unwrap()
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let seats = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert!(Arc::ptr_eq(&seats[0], &seats[1]));
    assert_eq!(created.load(std::sync::atomic::Ordering::SeqCst), 1);
}
