//! macOS M1: pairing refusal, stale epoch, Stop and crash against a fake helper
//! that speaks the real socket protocol from this (ad-hoc signed) test process.
use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Fake {
    Honest,
    WrongToken,
    StaleReply,
    StaleGeneration,
    CrashOnRequest,
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
        if method != "heartbeat" && fake == Fake::CrashOnRequest {
            return;
        }
        let epoch = if fake == Fake::StaleReply && method != "heartbeat" {
            json!("0")
        } else {
            epoch.clone()
        };
        let generation = if fake == Fake::StaleGeneration && method != "heartbeat" {
            json!("0")
        } else {
            epoch.clone()
        };
        let result = json!({"surface_id":"macos-seat","generation":generation});
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
    wait_for(Duration::from_secs(5), &mut done)
}

fn wait_for(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
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
fn m1_request_failure_retires_without_another_owner_call() {
    for fake in [
        Fake::StaleReply,
        Fake::StaleGeneration,
        Fake::CrashOnRequest,
    ] {
        let (mut seat, root, _) = seat("always", fake);
        seat.start().unwrap();
        let dir = seat.link.as_ref().unwrap().dir.clone();
        assert!(state(&mut seat).is_err());
        // Keep the seat alive, without ready(), stop() or Drop rescuing cleanup.
        let removed = wait(|| !dir.exists());
        drop(seat);
        std::fs::remove_dir_all(root).unwrap();
        assert!(removed, "request-fenced link retained its rendezvous");
    }
}

const SUBPROCESS_TEST: &str =
    "runtime::kernel_browser_host::mac_helper::tests::m1_shutdown_subprocess";

// Child roles use the real admission and host shutdown paths. The kernel role
// exits immediately after shutdown, so test harness/Drop cannot drain reapers.
#[test]
fn m1_shutdown_subprocess() {
    let Ok(role) = std::env::var("CUMAC_SHUTDOWN_ROLE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("CUMAC_SHUTDOWN_ROOT").unwrap());
    if role == "helper" {
        let path = root.join("pair-path");
        assert!(wait_for(Duration::from_secs(15), || path.exists()));
        let dir = PathBuf::from(std::fs::read_to_string(path).unwrap());
        let bootstrap: Value =
            serde_json::from_slice(&std::fs::read(dir.join("bootstrap")).unwrap()).unwrap();
        let mut stream = UnixStream::connect(dir.join("s")).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"token":bootstrap["token"],"epoch":bootstrap["epoch"],"pid":std::process::id()})
        )
        .unwrap();
        // Cannot process Stop or EOF, and cannot expire its own lease.
        unsafe { libc::raise(libc::SIGSTOP) };
        panic!("suspended helper unexpectedly resumed");
    }
    assert_eq!(role, "kernel");
    let launch_root = root.clone();
    let launch: Launch = Box::new(move |dir| {
        std::fs::write(
            launch_root.join("pair-path"),
            dir.as_os_str().as_encoded_bytes(),
        )
        .unwrap();
        Ok(())
    });
    let mut seat = MacComputerHelper::new(root.join("h"), "always".into(), launch);
    seat.start().unwrap();
    let link = seat.link.as_ref().unwrap();
    std::fs::write(
        root.join("paired.json"),
        serde_json::to_vec(&json!({"pid":link.pid,"started":link.started,"dir":link.dir})).unwrap(),
    )
    .unwrap();
    if std::env::var_os("CUMAC_SHUTDOWN_STOP_FIRST").is_some() {
        seat.stop().unwrap();
    }
    let host = super::super::KernelBrowserHost::new(root.clone());
    host.register_computer_seat(
        crate::session::DEFAULT_LOCAL_USER_ID,
        Arc::new(Mutex::new(seat)),
    )
    .unwrap();
    host.shutdown().unwrap();
    std::process::exit(0);
}

#[test]
fn m1_kernel_shutdown_drains_unresponsive_helper_retirement() {
    shutdown_case(false);
}

#[test]
fn m1_kernel_shutdown_drains_previously_stopped_helper_retirement() {
    shutdown_case(true);
}

fn shutdown_case(stop_first: bool) {
    let root = std::env::temp_dir().join(format!("cumac-{:08x}", rand::random::<u32>()));
    std::fs::create_dir(&root).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", SUBPROCESS_TEST, "--nocapture"])
        .env("CUMAC_SHUTDOWN_ROOT", &root);
    // Like LaunchServices, the helper is outside the kernel's process tree.
    // The test retains both children so failures cannot leave a suspended helper.
    let mut helper = command
        .env("CUMAC_SHUTDOWN_ROLE", "helper")
        .spawn()
        .unwrap();
    command.env("CUMAC_SHUTDOWN_ROLE", "kernel");
    if stop_first {
        command.env("CUMAC_SHUTDOWN_STOP_FIRST", "1");
    } else {
        command.env_remove("CUMAC_SHUTDOWN_STOP_FIRST");
    }
    let mut kernel = command.spawn().unwrap();
    let exited = wait_for(Duration::from_secs(15), || {
        kernel.try_wait().unwrap().is_some()
    });
    if !exited {
        kernel.kill().unwrap();
    }
    let status = kernel.wait().unwrap();
    let dead = wait(|| helper.try_wait().unwrap().is_some());
    let paired = std::fs::read(root.join("paired.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    // Observe before test-owned cleanup can rescue either assertion.
    let removed = paired
        .as_ref()
        .and_then(|paired| paired["dir"].as_str())
        .is_some_and(|dir| !Path::new(dir).exists());
    if !dead {
        helper.kill().unwrap();
    }
    helper.wait().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    let paired = paired.expect("kernel did not record a paired helper");
    assert_eq!(paired["pid"], helper.id());
    assert!(
        paired["started"].is_array(),
        "paired process start identity unavailable"
    );
    assert!(
        exited && status.success(),
        "kernel shutdown did not complete"
    );
    assert!(dead, "paired suspended helper survived kernel exit");
    assert!(removed, "helper rendezvous survived kernel exit");
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
