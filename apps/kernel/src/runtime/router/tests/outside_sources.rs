//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17, ON-18): a grant follows the owner's notification
//! sources through the real router. Supplementary regressions, not live acceptance.
use super::outside_events::{command, request, run};
use super::*;
use crate::local::*;

#[test]
fn a_grant_cannot_follow_sources_before_delivery_exists() {
    run("outside-sources-unavailable", async {
        let worktree = crate::test_support::TestWorktree::new("outside-sources-unavailable");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let subscribe = request(
            serde_json::json!({"SubscribeEvents":{"targets":[],"sources":["wf_source_a"]}}),
        );
        match router
            .dispatch(command(&grant, &subscribe), subscribe)
            .await
        {
            Err(DaemonError::MachineRequest { code, message }) => {
                assert_eq!(code, "events_refused");
                assert_eq!(message, "source not available");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    });
}
