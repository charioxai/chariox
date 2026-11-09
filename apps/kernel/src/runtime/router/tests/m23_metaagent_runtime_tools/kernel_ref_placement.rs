// MP-08 / MP-09 / MP-10 / MP-11: a Room creation and a batch spawn read a kernel reference the way
// `agent spawn --machine` does: id, alias or machine name; this kernel runs it here, another one
// goes to remote placement, an unknown or ambiguous name is refused and creates nothing.
use super::subtree::{world, World};
use super::*;
use crate::agent::CreateAgentRequest;
use crate::runtime::state::peer_agent_outbound::peer_agent_link_for_tests;

fn register(kernel: &str, name: &str) {
    peer_agent_link_for_tests::register(
        kernel,
        name,
        std::sync::Arc::new(|_, _, _| panic!("placement never talks to a peer here")),
    );
}

/// What each entry point answers for `kernel_ref`: the created agent's remote execution, or the refusal.
struct Outcome {
    session: Result<bool, String>,
    batch: Result<bool, String>,
    sessions_created: usize,
    agents_created: usize,
}

async fn place(world: &World, kernel_ref: &str) -> Outcome {
    let runtime = world.router.runtime_state();
    let (sessions_before, agents_before) = counts(world).await;
    let workspace = world._env.root.join("workspace").display().to_string();
    let session = runtime
        .create_session_response(
            crate::session::CreateSessionRequest::new(&workspace, &workspace)
                .with_kernel_ref(kernel_ref),
        )
        .await
        .map(|response| match response {
            LocalDaemonResponse::SessionCreated { agent, .. } => agent.remote_execution().is_some(),
            other => panic!("unexpected {other:?}"),
        })
        .map_err(|error| error.to_string());
    let batch = runtime
        .spawn_agents(
            vec![CreateAgentRequest::new(&world.room, "dev-stub")
                .with_worktree(&workspace)
                .with_kernel(kernel_ref)],
            "owner",
            &[None],
        )
        .await
        .map(|agents| agents[0].remote_execution().is_some())
        .map_err(|error| error.to_string());
    let (sessions, agents) = counts(world).await;
    Outcome {
        session,
        batch,
        sessions_created: sessions - sessions_before,
        agents_created: agents - agents_before,
    }
}

async fn counts(world: &World) -> (usize, usize) {
    let app = world.app.lock().await;
    let sessions = app.sessions().list_sessions();
    let agents = sessions
        .iter()
        .map(|session| app.agents().get_session_agents(session.id()).len())
        .sum();
    (sessions.len(), agents)
}

#[test]
fn a_room_or_batch_spawn_naming_the_own_machine_runs_here() {
    run_large_stack_async_test("kernel-ref-own-machine", own_machine);
}

async fn own_machine() {
    let world = world("kernel-ref-own-machine").await;
    let own = world.app.lock().await.config().daemon_id.clone();
    register(&own, "own-machine");
    let named = place(&world, "own-machine").await;
    let by_id = place(&world, &own).await;
    peer_agent_link_for_tests::unregister(&own);
    for (how, outcome) in [("machine name", named), ("kernel id", by_id)] {
        assert_eq!(outcome.session, Ok(false), "session create by {how}");
        assert_eq!(outcome.batch, Ok(false), "batch spawn by {how}");
        assert_eq!((outcome.sessions_created, outcome.agents_created), (1, 2));
    }
}

#[test]
fn a_room_or_batch_spawn_naming_another_machine_goes_to_placement_like_its_id() {
    run_large_stack_async_test("kernel-ref-other-machine", other_machine);
}

async fn other_machine() {
    let world = world("kernel-ref-other-machine").await;
    register("other-kernel", "other-machine");
    let named = place(&world, "other-machine").await;
    let by_id = place(&world, "other-kernel").await;
    peer_agent_link_for_tests::unregister("other-kernel");
    let again = |text: &Result<bool, String>, name: &str| {
        text.clone()
            .expect_err("no relay here: placement refuses")
            .replace(name, "<ref>")
    };
    assert_eq!(
        again(&named.session, "other-machine"),
        again(&by_id.session, "other-kernel")
    );
    assert_eq!(
        again(&named.batch, "other-machine"),
        again(&by_id.batch, "other-kernel")
    );
    assert!(!again(&named.session, "other-machine").contains("kernel_ambiguous"));
    assert_eq!((named.sessions_created, named.agents_created), (0, 0));
}

#[test]
fn a_room_or_batch_spawn_naming_an_unknown_machine_is_refused_and_creates_nothing() {
    run_large_stack_async_test("kernel-ref-unknown-machine", unknown_machine);
}

async fn unknown_machine() {
    let world = world("kernel-ref-unknown-machine").await;
    let outcome = place(&world, "unknown-xyz").await;
    assert!(
        outcome.session.is_err() && outcome.batch.is_err(),
        "refused"
    );
    assert_eq!((outcome.sessions_created, outcome.agents_created), (0, 0));
}

#[test]
fn a_room_or_batch_spawn_naming_an_ambiguous_machine_is_refused_and_creates_nothing() {
    run_large_stack_async_test("kernel-ref-ambiguous-machine", ambiguous_machine);
}

async fn ambiguous_machine() {
    let world = world("kernel-ref-ambiguous-machine").await;
    register("twin-one", "twin-machine");
    register("twin-two", "twin-machine");
    let outcome = place(&world, "twin-machine").await;
    peer_agent_link_for_tests::unregister("twin-one");
    peer_agent_link_for_tests::unregister("twin-two");
    for (entry, answer) in [("session", outcome.session), ("batch", outcome.batch)] {
        let refusal = answer.expect_err("an ambiguous machine is refused");
        assert!(refusal.contains("kernel_ambiguous"), "{entry}: {refusal}");
    }
    assert_eq!((outcome.sessions_created, outcome.agents_created), (0, 0));
}
