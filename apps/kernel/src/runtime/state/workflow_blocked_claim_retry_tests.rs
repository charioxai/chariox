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

/// 20A7 15:10Z (row 689): a workflow launch planned its run with the prompt owner held, then
/// waited for the session store's write lock, while a settlement held the session store and
/// waited for the prompt owner. Neither is a watched lock, so the kernel stopped silently.
/// The launch now follows the kernel's lock order: sessions, then prompts.
#[test]
fn a_launch_never_holds_the_prompt_owner_while_it_waits_for_the_session_store() {
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
    let (planned, launch_planned) = mpsc::channel();
    let (holding_sessions, sessions_held) = mpsc::channel::<()>();
    let (launch_done, launch_finished) = mpsc::channel();
    let launch = {
        let owned = owned.clone();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "model",
        )
        .with_agent_id(agent_id.clone());
        std::thread::spawn(move || {
            super::provider::AFTER_LAUNCH_PLANNED.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    planned.send(()).unwrap();
                    let _ = sessions_held.recv_timeout(Duration::from_secs(10));
                    // Let the settlement reach the prompt owner before the pointer is set.
                    std::thread::sleep(Duration::from_millis(300));
                }));
            });
            let result = owned.start_provider_launch(request);
            launch_done
                .send(result.map(|_| ()).map_err(|e| e.to_string()))
                .unwrap();
        })
    };
    launch_planned
        .recv_timeout(Duration::from_secs(10))
        .expect("the launch should plan its run");
    let (settle_done, settle_finished) = mpsc::channel();
    let settlement = {
        let owned = owned.clone();
        let agent_id = agent_id.clone();
        std::thread::spawn(move || {
            let sessions = owned.session_store.write();
            let session = sessions.get_session(session.id()).expect("the Room exists");
            holding_sessions.send(()).unwrap();
            let _ = owned.prompt_state_owner.state_parts(&session, &agent_id);
            drop(sessions);
            settle_done.send(()).unwrap();
        })
    };
    assert!(
        finished(&settle_finished).is_some(),
        "a settlement holding the session store waits for ever on the launch's prompt owner"
    );
    assert_eq!(
        finished(&launch_finished),
        Some(Ok(())),
        "the launch waits for ever on the session store"
    );
    launch.join().unwrap();
    settlement.join().unwrap();
}
