//! MP-10: a relay viewer's browser actor ends with the viewer, like a local socket's, except for a Tab its person took over.
#![cfg(unix)]
use super::support::*;
use crate::transport::relay_client::subscriptions::{
    abort_subscription_tasks, handle_relay_subscribe, handle_relay_unsubscribe,
    RelaySubscriptionTasks,
};
use chariox_relay::auth::RelaySubjectKind;
use chariox_relay::protocol::RelayCallerIdentity;

const TAB: &str = "host-tab-fixture";

struct Rig {
    router: Arc<CommandRouter>,
    tasks: RelaySubscriptionTasks,
    events: Arc<RelayEventRuntime>,
    session_id: String,
    attachment_id: String,
    user: String,
    _root: crate::test_support::TestWorktree,
}

impl Rig {
    async fn new() -> Self {
        let root = crate::test_support::TestWorktree::new("mp10-relay-actors");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session_id = create_test_session(&mut app, "workspace", "worktree");
        let attachment_id = attach_test_client(
            &mut app,
            &session_id,
            "viewer-attachment",
            ClientCapabilityLevel::MessageTransport,
        );
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        ));
        let rig = Self {
            user: Self::context(&router, 0).0,
            router,
            tasks: Default::default(),
            events: Arc::new(RelayEventRuntime::for_tests(64)),
            session_id,
            attachment_id,
            _root: root,
        };
        rig.router
            .runtime_state()
            .install_kernel_browser_fixture(&rig.user, rig._root.path());
        rig
    }

    fn identity(viewer: usize) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm".into(),
            subject: format!("viewer-{viewer}"),
            subject_kind: RelaySubjectKind::Client,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
            public_key_thumbprint: Some(format!("thumbprint-{viewer}")),
        }
    }

    fn caller(viewer: usize) -> crate::runtime::command::KernelCaller {
        crate::runtime::command::KernelCaller::for_relay_request(Some(Self::identity(viewer)))
    }

    fn command(viewer: usize, command: crate::local::KernelBrowserCommand) -> KernelCommand {
        let request =
            crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                command,
            });
        KernelCommand::from_local_request_with_caller(
            format!("viewer-{viewer}"),
            crate::runtime::command::KernelCommandSource::RelayClient,
            Self::caller(viewer),
            None,
            None,
            &request,
        )
    }

    fn context(router: &CommandRouter, viewer: usize) -> (String, String) {
        router
            .runtime_state()
            .kernel_browser_terminal_context(&Self::command(
                viewer,
                crate::local::KernelBrowserCommand::State,
            ))
            .unwrap()
    }

    fn actors(&self) -> usize {
        self.router
            .runtime_state()
            .kernel_browser_fixture_actors(&self.user)["actors"]
            .as_array()
            .unwrap()
            .len()
    }

    /// The Tabs a person holds after Take over.
    fn held_tabs(&self) -> usize {
        self.router
            .runtime_state()
            .kernel_browser_fixture_actors(&self.user)["input_ownership"]
            .as_array()
            .unwrap()
            .len()
    }

    /// The viewer opens its relay subscription, then takes over the Tab, as the web viewer does.
    async fn connect(&self, viewer: usize) -> Result<String, DaemonError> {
        let key = relay_crypto::public_key_from_private_key_base64(
            &relay_crypto::generate_private_key_base64(),
        )
        .unwrap();
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(4096);
        handle_relay_subscribe(
            &self.router,
            &outgoing,
            &self.tasks,
            &self.events,
            format!("request-{viewer}"),
            format!("subscription-{viewer}"),
            self.session_id.clone(),
            self.attachment_id.clone(),
            Some(Self::identity(viewer)),
            key.clone(),
            None,
            None,
            None,
        )
        .await?;
        let state = self.router.runtime_state();
        for step in [
            crate::local::KernelBrowserCommand::State,
            crate::local::KernelBrowserCommand::DisplayTakeover {
                tab_id: TAB.into(),
                generation: 1,
            },
        ] {
            state
                .kernel_browser_terminal_request(&Self::command(viewer, step.clone()), step)
                .await?;
        }
        Ok(key)
    }

    /// The relay reports the viewer's subscription gone, which it does for a closed tab and a lost link alike.
    async fn end(&self, viewer: usize, key: String) {
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(16);
        handle_relay_unsubscribe(
            &self.router,
            &outgoing,
            &self.tasks,
            format!("end-{viewer}"),
            format!("subscription-{viewer}"),
            key,
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn mp10_reloaded_relay_viewers_leave_only_the_actor_that_holds_the_takeover() {
    let _guard = relay_client_test_guard().await;
    let rig = Rig::new().await;
    let (mut left_behind, mut refused) = (Vec::new(), Vec::new());
    for viewer in 1..=70 {
        match rig.connect(viewer).await {
            Ok(key) => rig.end(viewer, key).await,
            Err(error) => refused.push(format!("viewer {viewer}: {error}")),
        }
        left_behind.push(rig.actors());
    }
    let held = rig.held_tabs();
    rig.router.runtime_state().shutdown_cleanup().await.unwrap();
    assert!(
        refused.is_empty() && left_behind.iter().all(|actors| *actors <= 1),
        "actors left after each of 70 ended viewers: {left_behind:?}; refused: {refused:?}"
    );
    assert_eq!(
        held, 1,
        "the person's takeover follows each reloaded viewer and is still theirs"
    );
}

#[tokio::test]
async fn mp10_a_viewer_with_two_subscriptions_keeps_its_actor_until_the_last_one_ends() {
    let _guard = relay_client_test_guard().await;
    let rig = Rig::new().await;
    let key = rig.connect(1).await.unwrap();
    let (outgoing, _priority, _events) = RelayOutgoingSender::channel(4096);
    handle_relay_subscribe(
        &rig.router,
        &outgoing,
        &rig.tasks,
        &rig.events,
        "request-second".into(),
        "subscription-second".into(),
        rig.session_id.clone(),
        rig.attachment_id.clone(),
        Some(Rig::identity(1)),
        key.clone(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    rig.end(1, key.clone()).await;
    assert_eq!(rig.actors(), 1, "its other subscription is still open");
    let (outgoing, _priority, _events) = RelayOutgoingSender::channel(16);
    handle_relay_unsubscribe(
        &rig.router,
        &outgoing,
        &rig.tasks,
        "end-second".into(),
        "subscription-second".into(),
        key,
    )
    .await
    .unwrap();
    assert_eq!(
        (rig.actors(), rig.held_tabs()),
        (1, 1),
        "the viewer is gone; its person still holds the Tab"
    );
    rig.router.runtime_state().shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn mp10_a_lost_relay_connection_keeps_the_persons_takeover_for_their_next_viewer() {
    let _guard = relay_client_test_guard().await;
    let rig = Rig::new().await;
    rig.connect(1).await.unwrap();
    assert_eq!(rig.actors(), 1);
    abort_subscription_tasks(&rig.router, &rig.tasks).await;
    assert_eq!(
        (rig.actors(), rig.held_tabs()),
        (1, 1),
        "the kernel lost its relay: no viewer is left, the person's takeover is kept"
    );
    rig.connect(2)
        .await
        .expect("the next viewer continues the person's takeover");
    assert_eq!((rig.actors(), rig.held_tabs()), (1, 1));
    rig.router.runtime_state().shutdown_cleanup().await.unwrap();
}
