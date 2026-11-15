// MP-08 / MP-09 / MP-10 / MP-11: a Room `agent spawn` on another kernel that carries a deadline
// leases the agent and has the worker enforce the deadline.
use super::subtree::{room_command, world_with_config};
use super::*;
use crate::runtime::state::kernel_access::test_support::worker_spy::WorkerSpy;
use std::sync::atomic::Ordering;

#[test]
fn a_remote_spawn_with_a_deadline_asks_the_worker_to_enforce_it() {
    run_large_stack_async_test("remote-spawn-deadline-armed", armed_on_worker);
}

async fn armed_on_worker() {
    let worker = WorkerSpy::new(false);
    let world = world_with_config("remote-spawn-deadline-armed", |config| {
        config.relay_url = Some(worker.url.clone());
        config.relay_token = Some("remote-spawn-deadline".into());
    })
    .await;
    for (alias, target) in [
        ("far-machine", format!("--machine {}", worker.id)),
        ("far-kernel", format!("--kernel {}", worker.id)),
    ] {
        let spawned = room_command(
            &world,
            "a",
            &format!("agent spawn {alias} --provider dev-stub {target} --deadline 30m"),
        )
        .await;
        assert!(spawned.ok, "{spawned:?}");
        let agent = &spawned.payload["response"]["agent"];
        let deadline_ms = agent["deadline_ms"]
            .as_u64()
            .unwrap_or_else(|| panic!("the remote child carries its deadline: {agent}"));
        let asked = worker.deadlines.lock().unwrap().clone();
        assert_eq!(
            asked.last().and_then(|(_, ms)| *ms),
            Some(deadline_ms),
            "the worker was asked for the deadline the home recorded: {asked:?}"
        );
    }
    assert_eq!(worker.deadlines.lock().unwrap().len(), 2);
}

#[test]
fn a_remote_spawn_without_a_deadline_still_leases_and_a_local_one_with_a_deadline_still_arms() {
    run_large_stack_async_test("remote-spawn-deadline-unchanged", unchanged);
}

async fn unchanged() {
    let worker = WorkerSpy::new(false);
    let world = world_with_config("remote-spawn-deadline-unchanged", |config| {
        config.relay_url = Some(worker.url.clone());
        config.relay_token = Some("remote-spawn-deadline".into());
    })
    .await;
    let remote = room_command(
        &world,
        "a",
        &format!(
            "agent spawn far --provider dev-stub --machine {}",
            worker.id
        ),
    )
    .await;
    assert!(remote.ok, "{remote:?}");
    let agent = &remote.payload["response"]["agent"];
    assert!(agent["deadline_ms"].is_null(), "{agent}");
    assert!(
        worker.requests.load(Ordering::SeqCst) > 0,
        "a remote spawn without a deadline leases on the worker"
    );
    let remote_agent = world
        .app
        .lock()
        .await
        .agents()
        .get_agent(agent["id"].as_str().unwrap())
        .unwrap();
    assert!(remote_agent.remote_execution().is_some());

    let requests = worker.requests.load(Ordering::SeqCst);
    let local = room_command(
        &world,
        "a",
        "agent spawn near --provider dev-stub --deadline 30m",
    )
    .await;
    assert!(local.ok, "{local:?}");
    assert!(
        local.payload["response"]["agent"]["deadline_ms"].is_u64(),
        "{local:?}"
    );
    assert_eq!(worker.requests.load(Ordering::SeqCst), requests);
}
