//! MP-08 / MP-09 / MP-10 / MP-11: the maintenance pump as a whole, with every await that used to
//! hold it stuck at once: the lifecycle sweep (a goal check, a failed workflow, a cancellation and
//! a peer follow on a silent worker, a silent Cloud directory), the cleanup of a failed provider
//! launch whose lane is held, and a trigger whose steer waits for the silent worker.
use super::*;
use crate::runtime::state::pump_stall_tests::{
    add_due_trigger, insert_run, retry_pending, schedule_launch_failure_retry, trigger,
};
use crate::session::AgentTriggerBusyPolicy;

/// With nothing to wait for the pump takes well under a second.
const WHOLE_PUMP_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pump_returns_while_the_sweep_a_launch_retry_and_a_trigger_are_all_stuck() {
    let f = Stalled::new(true).await;
    f.fail_workflow_run_of_child_task();
    f.expire_child_goal_check();
    let _directory = f.follow_a_peer_while_the_cloud_directory_is_silent();
    f.bystander();

    let held_run_id = "provider-run-whole-pump-held";
    let free_run_id = "provider-run-whole-pump-free";
    for run_id in [held_run_id, free_run_id] {
        insert_run(&f.runtime, &f.room, &f.parent, run_id);
        schedule_launch_failure_retry(&f.runtime, run_id);
    }
    let held = f.runtime.provider_runtime_lanes.acquire(held_run_id).await;
    add_due_trigger(
        &f.runtime,
        &f.room,
        &f.child,
        "stuck",
        AgentTriggerBusyPolicy::Steer,
    );
    add_due_trigger(
        &f.runtime,
        &f.room,
        &f.parent,
        "other",
        AgentTriggerBusyPolicy::Queue,
    );

    let started = std::time::Instant::now();
    tokio::time::timeout(WHOLE_PUMP_BOUND, f.runtime.pump_transport_runtime())
        .await
        .expect("the pump waits for the sweep, a launch retry or a trigger");
    eprintln!("whole pump returned after {:?}", started.elapsed());

    // Every part that does not depend on the silent worker or the held lane is done meanwhile.
    until(|| f.bystander_status() == "cancelled")
        .await
        .expect("the sweep did not supervise the bystander");
    until(|| !retry_pending(&f.runtime, free_run_id))
        .await
        .expect("the retry of the free run did not run");
    until(|| {
        let other = trigger(&f.runtime, &f.room, "other");
        other.fired() == 1 && !other.dispatch_in_flight()
    })
    .await
    .expect("the other trigger did not fire");
    // The stuck parts are still stuck, and kept.
    assert!(retry_pending(&f.runtime, held_run_id));
    assert!(trigger(&f.runtime, &f.room, "stuck").dispatch_in_flight());

    // Pumping again neither starts the stuck work twice nor holds the pump.
    for _ in 0..3 {
        tokio::time::timeout(WHOLE_PUMP_BOUND, f.runtime.pump_transport_runtime())
            .await
            .expect("a later pump waits");
    }
    assert!(retry_pending(&f.runtime, held_run_id));
    drop(held);
    until(|| !retry_pending(&f.runtime, held_run_id))
        .await
        .expect("the retry runs once its lane is free");
}
