use super::*;

// Row 551: restarting a kernel is a hot reload of it. Only its owner, the owner's agents and the owner's other kernels may ask;
// a refused hot reload answers with its reason and the kernel keeps running.
const OWNER_ONLY: &str = "Only the owner of this kernel or its agents can restart it.";

fn restart_request() -> LocalDaemonRequest {
    serde_json::from_value(serde_json::json!({"RestartKernel": {}}))
        .expect("RestartKernel is a kernel request")
}

fn router() -> CommandRouter {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1)
}

fn as_terminal(mut command: KernelCommand, user: &str) -> KernelCommand {
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    command.caller.user_id = Some(user.into());
    command
}

#[tokio::test]
async fn a_person_who_does_not_own_the_kernel_cannot_restart_it() {
    let router = router();
    let request = restart_request();
    let command = as_terminal(
        KernelCommand::from_local_request("restart-stranger", None, None, &request),
        "someone-else",
    );
    let refused = router.dispatch(command, request).await.unwrap_err();
    assert!(
        refused.to_string().contains(OWNER_ONLY),
        "the refusal says who may restart a kernel: {refused}"
    );
}

#[tokio::test]
async fn the_owner_is_told_why_a_hot_reload_cannot_start_and_the_kernel_keeps_running() {
    let router = router();
    let request = restart_request();
    let command = as_terminal(
        KernelCommand::from_local_request("restart-owner", None, None, &request),
        crate::session::DEFAULT_LOCAL_USER_ID,
    );
    // A test process was not started as a kernel program: the hot reload cannot be prepared, and says so.
    let refused = router.dispatch(command, request).await.unwrap_err();
    let text = refused.to_string();
    assert!(
        !text.contains(OWNER_ONLY),
        "the owner is not refused as a stranger: {text}"
    );
    assert!(
        text.contains("hot reload") && text.contains("It keeps running"),
        "the refusal carries the hot reload's own reason and says the kernel keeps running: {text}"
    );
}

#[tokio::test]
async fn an_agent_of_the_owner_is_admitted_like_the_owner() {
    let router = router();
    let request = restart_request();
    let mut command = KernelCommand::from_local_request("restart-agent", None, None, &request);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::KernelAgent);
    let refused = router.dispatch(command, request).await.unwrap_err();
    assert!(
        !refused.to_string().contains(OWNER_ONLY),
        "an agent of the owner is not refused as a stranger: {refused}"
    );
}

#[tokio::test]
async fn the_restart_tool_serves_only_a_kernels_own_agents_and_names_a_bad_argument() {
    let router = router();
    let state = router.runtime_state();
    let stranger = state
        .kernel_restart_tool("no-such-run", serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(
        stranger.to_string().contains(OWNER_ONLY),
        "a caller with no run on this kernel is refused: {stranger}"
    );
    let bad = state
        .kernel_restart_tool("no-such-run", serde_json::json!({"nonsense": 1}))
        .await
        .unwrap_err();
    assert!(
        bad.to_string().contains("one optional argument"),
        "an unknown argument is named: {bad}"
    );
    assert_eq!(
        crate::runtime::state::kernel_restart_spec().name,
        "chariox.kernels.restart"
    );
}

#[tokio::test]
async fn an_agent_of_another_person_is_refused_at_the_restart_gate() {
    let router = router();
    // The agent classes a caller is admitted on say "an agent", not whose: the gate also checks the agent's person.
    for (class, metaagent) in [
        (crate::local::KernelConnectionClass::KernelAgent, None),
        (crate::local::KernelConnectionClass::ExternalAgent, None),
        (
            crate::local::KernelConnectionClass::KernelAgent,
            Some("agent-of-someone-else"),
        ),
    ] {
        let request = restart_request();
        let mut command =
            KernelCommand::from_local_request("restart-foreign-agent", None, None, &request);
        command.caller = command.caller.with_connection_class(class);
        command.caller.user_id = Some("someone-else".into());
        command.caller.metaagent_id = metaagent.map(str::to_owned);
        let refused = router
            .dispatch(command, request)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            !refused.contains("hot reload"),
            "an agent of another person ({class:?}) never gets as far as the hot reload: {refused}"
        );
        // An external agent with no grant here is already stopped by kernel access; a kernel's own agent reaches this gate.
        if class == crate::local::KernelConnectionClass::KernelAgent {
            assert!(
                refused.contains(OWNER_ONLY),
                "an agent of another person is refused as a stranger: {refused}"
            );
        }
    }
}
