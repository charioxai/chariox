//! MP-08/MP-09/MP-10/MP-11: ending worker leases must leave the home app available.
use super::tests::fixture_with_options;
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unanswered_worker_cleanup_leaves_the_app_available() {
    crate::test_support::isolated_env_test!();
    let f = fixture_with_options(None, true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    super::forgotten_machine_tests::lease_agent_to_machine(&f, "unanswered-worker");
    let agent_id = f.request.target_agent_id.as_deref().unwrap();
    let mut binding = f
        .state
        .owned
        .agent_store
        .get_agent(agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    binding.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
    binding.relay_token = Some("unanswered-relay-fixture".into());
    let agent = f
        .state
        .owned
        .agent_store
        .bind_remote_execution(agent_id, binding)
        .unwrap();
    let state = f.state.clone();
    let cleanup = tokio::spawn(async move { state.release_worker_leases(vec![agent]).await });
    let (socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();

    // Hold the relay handshake open: the background cleanup is waiting on I/O.
    let available = tokio::time::timeout(
        Duration::from_millis(250),
        f.state.with_app_side_effect(|_| ()),
    )
    .await;
    drop(socket);
    drop(listener);
    tokio::time::timeout(Duration::from_secs(5), cleanup)
        .await
        .unwrap()
        .unwrap();
    assert!(
        available.is_ok(),
        "an unanswered worker cleanup must not hold the app lock"
    );
}
