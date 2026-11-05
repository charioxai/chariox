//! MP-08 / MP-09 / MP-10 / MP-11: a provider launch holds the prompt owner's lock and then takes
//! the session store's write lock, so the claim retry must not ask the owner while it holds the
//! store's read guard. Before, the two waited for each other for ever at kernel start.
use crate::test_support::allocation::runtime_with_rooms;
use std::sync::mpsc;
use std::time::Duration;

fn finished<T>(receiver: &mpsc::Receiver<T>) -> Option<T> {
    receiver.recv_timeout(Duration::from_secs(10)).ok()
}

#[test]
fn a_claim_retry_does_not_wait_on_the_prompt_owner_under_the_session_read_guard() {
    let runtime = runtime_with_rooms(1);
    let owned = runtime.owned.clone();
    let session = owned
        .session_store
        .list_all_sessions()
        .pop()
        .expect("the Room exists");
    let agent_id = owned
        .agent_store
        .get_session_agents(session.id())
        .pop()
        .expect("the Room has an agent")
        .id()
        .to_string();
    let (holding_owner, owner_held) = mpsc::channel();
    let (launch_done, launch_finished) = mpsc::channel();
    let launch = {
        let owned = owned.clone();
        let session_id = session.id().to_string();
        std::thread::spawn(move || {
            let result =
                owned
                    .prompt_state_owner
                    .with_unarchived_agent(&session, &agent_id, || {
                        holding_owner.send(()).unwrap();
                        // Let the retry reach the owner before asking for the write lock.
                        std::thread::sleep(Duration::from_millis(500));
                        owned
                            .session_store
                            .set_active_provider_run(&session_id, None)
                    });
            launch_done.send(result.is_ok()).unwrap();
        })
    };
    owner_held.recv().unwrap();
    let (retry_done, retry_finished) = mpsc::channel();
    let retry = {
        let owned = owned.clone();
        std::thread::spawn(move || {
            owned.workflow_retry_blocked_claims();
            retry_done.send(()).unwrap();
        })
    };
    assert!(
        finished(&launch_finished).is_some(),
        "the launch waits for the session store while the retry waits for the prompt owner"
    );
    assert!(
        finished(&retry_finished).is_some(),
        "the claim retry never finished"
    );
    launch.join().unwrap();
    retry.join().unwrap();
}
