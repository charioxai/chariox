//! macOS M1: pairing refusal, stale epoch, Stop and crash against a fake helper
//! that speaks the real socket protocol from this (ad-hoc signed) test process.
use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Fake {
    Honest,
    WrongToken,
    StaleReply,
    CrashOnHeartbeat,
}

fn seat(requirement: &str, fake: Fake) -> (MacComputerHelper, PathBuf, Arc<Mutex<Vec<String>>>) {
    let root = std::env::temp_dir().join(format!("cumac-{:08x}", rand::random::<u32>()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let launch: Launch = Box::new(move |dir| {
        let (dir, log) = (dir.to_path_buf(), log.clone());
        std::thread::spawn(move || helper(&dir, fake, &log));
        Ok(())
    });
    (
        MacComputerHelper::new(root.join("h"), requirement.into(), launch),
        root,
        seen,
    )
}

fn helper(dir: &Path, fake: Fake, seen: &Mutex<Vec<String>>) {
    let bootstrap: Value =
        serde_json::from_slice(&std::fs::read(dir.join("bootstrap")).unwrap()).unwrap();
    let mut stream = UnixStream::connect(dir.join("s")).unwrap();
    let token = if fake == Fake::WrongToken {
        "0".repeat(64)
    } else {
        bootstrap["token"].as_str().unwrap().into()
    };
    let epoch = bootstrap["epoch"].clone();
    writeln!(
        stream,
        "{}",
        json!({"token":token,"epoch":epoch,"pid":std::process::id()})
    )
    .unwrap();
    let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
    while let Some(Ok(line)) = lines.next() {
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap().to_string();
        seen.lock().unwrap().push(method.clone());
        if method == "heartbeat" && fake == Fake::CrashOnHeartbeat {
            return;
        }
        let epoch = if fake == Fake::StaleReply {
            json!("0")
        } else {
            epoch.clone()
        };
        let result = json!({"surface_id":"macos-seat","generation":epoch});
        writeln!(
            stream,
            "{}",
            json!({"id":request["id"],"epoch":epoch,"ok":true,"result":result})
        )
        .unwrap();
        if method == "stop" {
            return;
        }
    }
}

fn wait(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

fn state(seat: &mut MacComputerHelper) -> Result<Value, HostFailure> {
    seat.request("host.computer", json!({"op":"state"}), None)
}

#[test]
fn m1_pairing_refuses_foreign_code_identity_and_wrong_token() {
    let foreign = format!(
        "identifier \"{HELPER_ID}\" and cdhash H\"{}\"",
        "0".repeat(40)
    );
    for (requirement, fake) in [
        (foreign.as_str(), Fake::Honest),
        ("always", Fake::WrongToken),
    ] {
        let (mut seat, root, seen) = seat(requirement, fake);
        assert!(seat.start().is_err());
        assert!(!seat.ready());
        // Refused before the helper sends anything beyond its hello.
        assert!(seen.lock().unwrap().is_empty());
        assert!(!root.join("h").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn m1_paired_seat_reports_kernel_epoch_and_stop_cleans_up() {
    let (mut seat, root, seen) = seat("always", Fake::Honest);
    seat.start().unwrap();
    let dir = seat.link.as_ref().unwrap().dir.clone();
    assert!(!dir.join("bootstrap").exists() && !dir.join("s").exists());
    let first = state(&mut seat).unwrap();
    assert_eq!(
        first["generation"],
        seat.link.as_ref().unwrap().epoch.as_str()
    );
    assert!(wait(|| seen
        .lock()
        .unwrap()
        .iter()
        .any(|m| m == "heartbeat")));
    seat.stop().unwrap();
    assert!(!seat.ready());
    assert!(wait(|| !dir.exists()));
    assert_eq!(seen.lock().unwrap().last().unwrap(), "stop");
    assert!(state(&mut seat).is_err());
    seat.start().unwrap();
    assert_ne!(state(&mut seat).unwrap()["generation"], first["generation"]);
    let dir = seat.link.as_ref().unwrap().dir.clone();
    drop(seat);
    assert!(wait(|| !dir.exists()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn m1_stale_epoch_reply_fences_the_seat() {
    let (mut seat, root, _) = seat("always", Fake::StaleReply);
    seat.start().unwrap();
    assert!(matches!(
        state(&mut seat),
        Err(HostFailure::Refused(UserDomainRefusalReason::StaleEpoch))
    ));
    assert!(!seat.ready());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn m1_helper_crash_is_observed_and_owned_state_is_removed() {
    let (mut seat, root, _) = seat("always", Fake::CrashOnHeartbeat);
    seat.start().unwrap();
    let dir = seat.link.as_ref().unwrap().dir.clone();
    // The watcher observes the lost lease without any client request.
    assert!(wait(|| !dir.exists()));
    assert!(!seat.ready());
    assert!(state(&mut seat).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn m1_old_reaper_cannot_remove_the_root_during_new_pairing_setup() {
    let (mut seat, root, _) = seat("always", Fake::Honest);
    seat.start().unwrap();
    let old = seat.link.as_ref().unwrap().clone();
    seat.stop().unwrap();
    assert!(wait(|| Arc::strong_count(&old) == 1));

    // Pause the same filesystem critical section used by pair() between parent
    // and child creation. A delayed old watcher/reaper must wait for that child.
    let guard = seat.rendezvous_guard.lock().unwrap();
    std::fs::create_dir_all(&seat.run_root).unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reaper = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        old.retire();
        done_tx.send(()).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let premature = done_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    let parent_survived = seat.run_root.is_dir();
    let fresh = seat.run_root.join("new-pairing");
    std::fs::create_dir_all(&fresh).unwrap();
    drop(guard);
    reaper.join().unwrap();
    let fresh_survived = fresh.is_dir();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        !premature && parent_survived && fresh_survived,
        "old reaping removed the empty parent during new pairing setup"
    );
}
