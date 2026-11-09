// MP-08 / MP-09 / MP-10 / MP-11: a Room `agent spawn` on another kernel that carries a deadline is
// refused before the worker hears of it, so a refused spawn leaves nothing on the worker.
use super::subtree::{error_text, room_command, world_with_config};
use super::*;
use crate::runtime::state::kernel_access::test_support::worker_spy::WorkerSpy;
use std::sync::atomic::Ordering;

#[test]
fn a_remote_spawn_with_a_deadline_is_refused_before_the_worker_is_asked() {
    run_large_stack_async_test("remote-spawn-deadline-refused", refused_before_lease);
}

async fn refused_before_lease() {
    let worker = WorkerSpy::new(false);
    let world = world_with_config("remote-spawn-deadline-refused", |config| {
        config.relay_url = Some(worker.url.clone());
        config.relay_token = Some("remote-spawn-deadline".into());
    })
    .await;
    let agents_before = room_agents(&world).await;
    for target in [
        format!("--machine {}", worker.id),
        format!("--kernel {}", worker.id),
    ] {
        let refused = room_command(
            &world,
            "a",
            &format!("agent spawn far --provider dev-stub {target} --deadline 30m"),
        )
        .await;
        assert!(!refused.ok, "{refused:?}");
        let message = error_text(&refused);
        assert!(
            message.contains("arm an agent deadline on its executing kernel"),
            "{message}"
        );
        assert!(
            message.contains("leased-agent deadlines require worker enforcement"),
            "{message}"
        );
        assert_eq!(
            worker.requests.load(Ordering::SeqCst),
            0,
            "the worker received a lease request for a spawn that cannot be armed"
        );
    }
    assert_eq!(
        room_agents(&world).await,
        agents_before,
        "no agent was created"
    );
}

async fn room_agents(world: &super::subtree::World) -> usize {
    world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room)
        .len()
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
