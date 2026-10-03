use super::*;
use crate::durable_state::app_state::{fixture_neighbour_catalog, AppStateOperation};
use std::sync::atomic::AtomicUsize;

#[test]
fn sustained_storage_queue_preserves_running_workers_authority() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    fixture_neighbour_catalog(&store);
    stage(&store);
    let native = Arc::new(NativeFixture::compile().unwrap());
    let (control, observations) = make_control(&store, native);
    let service = control.lifecycle();
    for installation in ["installed", "neighbour"] {
        service
            .start_active_blocking("alice", installation, runtime.handle().clone())
            .unwrap();
        wait(|| control.active_app_lease("alice", installation).is_some());
    }
    let stopping = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicUsize::new(0));
    let mut traffic = Vec::new();
    // Sixteen bounded operations continuously replenish the same eight-slot
    // pool used by SDK storage and periodic lifecycle verification. Each runs
    // the real state writer. Retain its permit for 2 ms to pin a queued load.
    for _ in 0..16 {
        let stopping = stopping.clone();
        let completed = completed.clone();
        let admission = service.0.admission.clone();
        let store = store.clone();
        let catalog = catalog.clone();
        traffic.push(runtime.spawn(async move {
            while !stopping.load(Ordering::Acquire) {
                let permit = admission.clone().acquire_owned().await.unwrap();
                let store = store.clone();
                let catalog = catalog.clone();
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    store
                        .execute_app_state(
                            "alice",
                            catalog,
                            AppStateOperation::Get {
                                key: "status".into(),
                            },
                            AppOperationBudget::from_supervisor(|| false),
                        )
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(2));
                })
                .await
                .unwrap();
                completed.fetch_add(1, Ordering::Release);
            }
        }));
    }
    wait(|| completed.load(Ordering::Acquire) >= 32);
    // Exceed the real fixed 30-second authority-check budget. Both the busy
    // installation and its idle neighbour must remain running throughout.
    let deadline = Instant::now() + Duration::from_secs(34);
    let mut healthy = true;
    while Instant::now() < deadline {
        healthy &= ["installed", "neighbour"].iter().all(|installation| {
            control.active_app_lease("alice", installation).is_some()
                && store
                    .app_worker_status("alice", installation)
                    .unwrap()
                    .is_some_and(|status| status.phase == WorkerPhase::Running)
        });
        std::thread::sleep(Duration::from_millis(100));
    }
    let statuses: Vec<_> = ["installed", "neighbour"]
        .iter()
        .map(|installation| {
            let status = store
                .app_worker_status("alice", installation)
                .unwrap()
                .unwrap();
            (installation, status.phase, status.failure)
        })
        .collect();
    println!("sustained storage status: {statuses:?}");
    let reads = completed.load(Ordering::Acquire);
    stopping.store(true, Ordering::Release);
    runtime.block_on(async {
        for task in traffic {
            task.await.unwrap();
        }
    });
    service.shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
    assert!(
        healthy,
        "healthy workers failed under sustained storage: {statuses:?}"
    );
    assert!(reads > 100, "sustained real storage work required");
    println!("lifecycle storage drill: completed_reads={reads}, duration_s=34, healthy_workers=2");
}
