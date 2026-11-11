use super::*;
use crate::runtime::process_resources::LaunchAdmission;
use std::sync::mpsc;
use std::time::Instant;

/// How long a stand-in for worker cleanup keeps the process-resources registry.
const REGISTRY_HELD: Duration = Duration::from_millis(1500);
/// The runtime must keep ticking this closely while the registry is held.
const RUNTIME_STALL_BOUND: Duration = Duration::from_millis(500);

#[tokio::test(flavor = "current_thread")]
async fn supervise_keeps_the_runtime_running_while_cleanup_holds_the_registry() {
    let dir = std::env::temp_dir().join(format!("supervise-lock-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let argv = vec!["/bin/sleep".to_string(), "30".to_string()];
    let intent = super::super::LaunchIntent {
        exec_id: "supervise-lock".into(),
        session_id: "supervise-lock".into(),
        owner_user_id: "owner".into(),
        acting_agent: None,
        cwd: dir.clone(),
        command: argv.clone(),
    };
    let admission = LaunchAdmission::limited(512 << 20, u64::MAX);
    let permit = admission
        .reserve("supervise-lock", &argv, &dir, false)
        .unwrap();
    let mut process = spawn(&dir, &argv, &permit, &intent).unwrap();
    let mut permit = Some(permit);
    let (cancel_tx, mut cancel) = watch::channel(None);

    // The holder releases by itself, so a stalled runtime cannot hang the test.
    let (held_tx, held_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        crate::runtime::process_resources::with_registry_held(|| {
            held_tx.send(()).unwrap();
            std::thread::sleep(REGISTRY_HELD);
        })
    });
    held_rx.recv_timeout(Duration::from_secs(10)).unwrap();

    let supervising = tokio::spawn(async move {
        let outcome = supervise(&mut process, &mut cancel, &mut permit).await;
        (outcome, permit)
    });
    let started = Instant::now();
    let mut last_tick = started;
    let mut widest_gap = Duration::ZERO;
    // Covers supervise's first sample (250 ms in) through most of the hold.
    while started.elapsed() < REGISTRY_HELD - Duration::from_millis(250) {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let now = Instant::now();
        widest_gap = widest_gap.max(now - last_tick);
        last_tick = now;
    }
    holder.join().unwrap();
    cancel_tx.send_replace(Some(CancelReason::Canceled));
    let (outcome, permit) = tokio::time::timeout(Duration::from_secs(10), supervising)
        .await
        .expect("supervise must end after cancel")
        .unwrap();
    drop(permit);
    std::fs::remove_dir_all(dir).unwrap();

    assert!(
        matches!(outcome, ExecOutcome::Signaled { .. }),
        "canceled sleep must be signaled, got {outcome:?}"
    );
    assert!(
        widest_gap < RUNTIME_STALL_BOUND,
        "the runtime stalled for {widest_gap:?} while supervise sampled with the registry held"
    );
}
