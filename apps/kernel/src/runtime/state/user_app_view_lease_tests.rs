//! A view the client renders itself ends when its client stops renewing the lease.
use super::*;
use crate::{
    runtime::{router::CommandRouter, user_app_views::CLIENT_LEASE},
    DaemonApp, DaemonConfig,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

fn runtime() -> KernelRuntimeState {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4).runtime_state()
}

/// Moves the virtual clock and lets the lease watchers it wakes run.
async fn pass(time: Duration) {
    tokio::time::advance(time).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

fn leased_view(runtime: &KernelRuntimeState) -> String {
    let views = runtime.app_control().user_views();
    let binding = AppViewBinding {
        owner: "alice".into(),
        installation: "installed".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    let view = views.open("alice", binding, "lease").unwrap();
    assert!(views.lease("alice", &view.view_id, Instant::now()));
    let (state, owner, id) = (runtime.clone(), "alice".to_owned(), view.view_id.clone());
    tokio::spawn(async move { state.close_user_app_view_at_lapse(&owner, &id).await });
    view.view_id
}

#[tokio::test(start_paused = true)]
async fn a_view_whose_client_stops_renewing_is_closed_and_a_renewed_one_is_kept() {
    let runtime = runtime();
    let views = runtime.app_control().user_views().clone();
    let (renewed, abandoned) = (leased_view(&runtime), leased_view(&runtime));
    pass(CLIENT_LEASE * 3 / 4).await;
    assert!(views.renew("alice", &renewed, Instant::now()));
    pass(CLIENT_LEASE / 2).await;
    assert!(
        views.get("alice", &abandoned).is_none(),
        "no renewal for a whole lease: the kernel ended the view"
    );
    assert!(
        views.get("alice", &renewed).is_some(),
        "its client renewed it inside the lease"
    );
    pass(CLIENT_LEASE).await;
    assert!(
        views.get("alice", &renewed).is_none(),
        "the renewals stopped, and so did the view"
    );
    assert!(views.list("alice").is_empty());
}

#[tokio::test(start_paused = true)]
async fn closing_a_view_ends_its_lease_watch_without_a_second_close() {
    let runtime = runtime();
    let views = runtime.app_control().user_views().clone();
    let view = leased_view(&runtime);
    assert!(views.close("alice", &view));
    pass(CLIENT_LEASE * 2).await;
    assert!(views.list("alice").is_empty());
    assert_eq!(views.lease_deadline(&view), None);
}
