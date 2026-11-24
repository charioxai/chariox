//! Local 1014: the pane feed through the router, as a provider terminal and the web pane read it.
use super::*;
use crate::local::{GetPaneFeedRequest, PaneFeed, PaneState, ReportPaneSettingsRequest};
use crate::transport::kernel_protocol::KernelEvent;

struct Pane {
    router: Arc<CommandRouter>,
    session_id: String,
    agent_id: String,
    config: DaemonConfig,
    _worktree: crate::test_temp_dir::TestTempDir,
}

fn pane(client_interface: crate::provider::ProviderClientInterface) -> Pane {
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon should boot");
    let worktree = crate::test_temp_dir::TestTempDir::new("pane-feed");
    let path = worktree.path().display().to_string();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(&path, &path))
        .expect("session should be created");
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id)
                .with_client_interface(client_interface),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        1,
    ));
    router.active_turns.start(crate::app::ActiveTurnState::new(
        session_id.clone(),
        agent_id.clone(),
        "prompt-1".to_string(),
        run.id().to_string(),
    ));
    Pane {
        router,
        session_id,
        agent_id,
        config,
        _worktree: worktree,
    }
}

fn question(id: &str, agent_id: &str) -> RuntimeInteraction {
    let allow = RuntimeInteractionChoice::new("allow", "Allow", "allow", None);
    RuntimeInteraction::new(
        id,
        agent_id,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        None,
        format!("Question {id}"),
        vec![
            allow,
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        None,
        None,
    )
}

impl Pane {
    async fn send(&self, request: LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        let mut command = KernelCommand::from_local_request("pane", None, None, &request);
        command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
        self.router.dispatch(command, request).await
    }

    async fn feed(&self) -> PaneFeed {
        let request = LocalDaemonRequest::GetPaneFeed(GetPaneFeedRequest {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
        });
        match self.send(request).await.expect("the pane feed reads") {
            LocalDaemonResponse::PaneFeed { feed } => feed,
            other => panic!("expected the pane feed, got {other:?}"),
        }
    }

    async fn answer(&self, interaction_id: &str, choice_id: &str) {
        let request =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: self.session_id.clone(),
                interaction_id: interaction_id.into(),
                choice_id: choice_id.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            });
        self.send(request).await.expect("the answer is taken");
    }
}

#[tokio::test]
async fn the_pane_feed_lists_a_card_a_text_terminal_can_answer_and_drops_it_once_answered() {
    let pane = pane(crate::provider::ProviderClientInterface::Chariox);
    let _pending = pane
        .router
        .runtime_state
        .create_runtime_interaction(&pane.session_id, question("ask", &pane.agent_id))
        .await
        .expect("the card is raised");

    let feed = pane.feed().await;
    assert_eq!(feed.identity.agent_id, pane.agent_id);
    assert_eq!(feed.state, PaneState::WaitingForAnswer);
    let cards = feed
        .cards
        .iter()
        .map(|card| (card.id.as_str(), card.text_terminal))
        .collect::<Vec<_>>();
    assert_eq!(
        cards,
        [("ask", true)],
        "choices are answered in the terminal"
    );
    assert!(feed.title.ends_with("· 1 card waiting"), "{}", feed.title);
    assert_eq!(feed.link, None, "a kernel with no pairing has no link");
    assert!(
        feed.cards[0]
            .open_elsewhere
            .contains(&feed.identity.agent_name),
        "with no link the line names the Room and agent: {}",
        feed.cards[0].open_elsewhere
    );

    pane.answer("ask", "allow").await;
    let feed = pane.feed().await;
    assert!(feed.cards.is_empty(), "an answered card leaves every pane");
    assert_ne!(feed.state, PaneState::WaitingForAnswer);
    assert!(!feed.title.contains("waiting"), "{}", feed.title);
}

#[tokio::test]
async fn a_watched_pane_feed_shows_a_card_answered_elsewhere_within_two_seconds() {
    use crate::transport::kernel_resource_feed::{
        run_kernel_resource_feed, submit_watch_set, watch_inbox,
    };
    let pane = pane(crate::provider::ProviderClientInterface::Chariox);
    let (watches, updates) = watch_inbox();
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(run_kernel_resource_feed(
        Arc::clone(&pane.router),
        crate::runtime::command::KernelCommandSource::RelayClient,
        updates,
        std::time::Duration::from_secs(60),
        move |event| {
            let sent = events_tx.send(event).is_ok();
            async move { sent }
        },
    ));
    let owner = crate::runtime::command::KernelCaller::for_relay_request(Some(
        chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "test".into(),
            subject: "client-owner".into(),
            subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
            public_key_thumbprint: None,
        },
    ));
    let watch = crate::local::KernelResourceWatch {
        resource: "pane_feed".into(),
        params: serde_json::json!({ "session_id": pane.session_id, "agent_id": pane.agent_id }),
        known_revision: None,
    };
    let refused = submit_watch_set(&watches, vec![watch], owner)
        .await
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    async fn next_cards(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<KernelEvent>,
    ) -> Vec<String> {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .expect("a pane change within 2 s, not the 10 s unsignalled interval")
                .unwrap();
            if let KernelEvent::KernelResourceChanged { body, .. } = event {
                let body = serde_json::to_value(body).unwrap();
                return body["snapshot"]["data"]["feed"]["cards"]
                    .as_array()
                    .expect("a pane feed snapshot")
                    .iter()
                    .map(|card| card["id"].as_str().unwrap().to_owned())
                    .collect();
            }
        }
    }
    assert!(next_cards(&mut events).await.is_empty());
    let _pending = pane
        .router
        .runtime_state
        .create_runtime_interaction(&pane.session_id, question("ask", &pane.agent_id))
        .await
        .unwrap();
    assert_eq!(next_cards(&mut events).await, ["ask"]);
    pane.answer("ask", "deny").await;
    assert!(next_cards(&mut events).await.is_empty());
}

#[tokio::test]
async fn only_a_provider_terminal_reports_settings_and_every_client_then_shows_them() {
    let report = |pane: &Pane| {
        LocalDaemonRequest::ReportPaneSettings(ReportPaneSettingsRequest {
            session_id: pane.session_id.clone(),
            agent_id: pane.agent_id.clone(),
            model: Some("opus".into()),
            effort: None,
            execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
        })
    };
    let web_run = pane(crate::provider::ProviderClientInterface::Chariox);
    let refused = web_run.send(report(&web_run)).await.unwrap_err();
    assert!(
        refused.to_string().contains("runs in no provider terminal"),
        "{refused}"
    );
    assert!(!web_run
        .router
        .runtime_state
        .provider_terminal_runs_agent(&web_run.session_id, &web_run.agent_id));

    let terminal = pane(crate::provider::ProviderClientInterface::NativeTui);
    assert!(terminal
        .router
        .runtime_state
        .provider_terminal_runs_agent(&terminal.session_id, &terminal.agent_id));
    let LocalDaemonResponse::PaneFeed { feed } = terminal.send(report(&terminal)).await.unwrap()
    else {
        panic!("a report answers with the pane feed");
    };
    assert_eq!(feed.settings.model.as_deref(), Some("opus"));
    assert_eq!(
        feed.settings.execution_mode,
        crate::provider::AgentExecutionMode::Plan
    );
    assert_eq!(
        feed.settings.permission_level,
        crate::provider::AgentPermissionLevel::Required
    );
    assert_eq!(
        terminal.feed().await.settings,
        feed.settings,
        "every reader sees it"
    );
}

#[tokio::test]
async fn reported_settings_survive_a_kernel_restart() {
    let terminal = pane(crate::provider::ProviderClientInterface::NativeTui);
    let report = LocalDaemonRequest::ReportPaneSettings(ReportPaneSettingsRequest {
        session_id: terminal.session_id.clone(),
        agent_id: terminal.agent_id.clone(),
        model: Some("opus".into()),
        effort: Some("high".into()),
        execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
        permission_level: Some(crate::provider::AgentPermissionLevel::Required),
    });
    terminal.send(report).await.expect("the report is taken");
    let Pane {
        router,
        agent_id,
        config,
        _worktree,
        ..
    } = terminal;
    drop(router);
    let restored = DaemonApp::bootstrap(config).expect("the kernel restarts on its state");
    let agent = restored
        .agents()
        .get_agent(&agent_id)
        .expect("the agent restores");
    assert_eq!(agent.model(), Some("opus"));
    assert_eq!(agent.effort(), Some("high"));
    assert_eq!(
        agent.execution_mode_override(),
        Some(crate::provider::AgentExecutionMode::Plan),
        "the terminal's mode is the agent's after a restart"
    );
    assert_eq!(
        agent.permission_level_override(),
        Some(crate::provider::AgentPermissionLevel::Required)
    );
}

/// A run of a person-responsible workflow fails two steps down (A -> B -> C, C needs help): the
/// attention stripe is in C's pane and names C; A's and B's panes have none.
#[tokio::test]
async fn a_failed_run_is_a_stripe_in_the_pane_of_the_agent_whose_step_failed() {
    use crate::session::{
        WorkflowFailureEvent, WorkflowFailureKind, WorkflowNodeRun, WorkflowNodeRunStatus,
        WorkflowRun, WorkflowRunStatus,
    };
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let mut session = crate::session::RuntimeSession::new(
        format!("pane-workflow-{:016x}", rand::random::<u64>()),
        crate::session::ProjectId::test(),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    session.set_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
    let session_id = session.id().to_owned();
    app.sessions_mut().restore_session(session);
    let agents = ["entry", "middle", "deploy"].map(|name| {
        let id = format!("{name}-{:016x}", rand::random::<u64>());
        app.agents_mut()
            .restore_agent(crate::agent::AgentInstance::new(
                id.clone(),
                name,
                session_id.clone(),
                Some(name.to_string()),
                "dev-stub",
                None,
                None,
                None,
                crate::agent::GridPosition::new(0, 0, 1, 1),
            ));
        id
    });
    let workflow = app
        .sessions_mut()
        .create_workflow(&session_id, Some("release-pipeline".into()))
        .unwrap();
    let mut session = app.sessions_mut().get_session(&session_id).unwrap();
    let steps = agents
        .iter()
        .zip(["node-a", "node-b", "node-c"])
        .enumerate()
        .map(|(index, (agent, node))| {
            let status = if index == 2 {
                WorkflowNodeRunStatus::Failed
            } else {
                WorkflowNodeRunStatus::Completed
            };
            WorkflowNodeRun::new(format!("node-run-{index}"), node, agent, 0, status)
        })
        .collect::<Vec<_>>();
    let mut run = WorkflowRun::new(
        "run-1",
        workflow.id(),
        "endpoint",
        "node-a",
        None,
        None,
        steps,
        vec![],
    )
    // Started by the entry agent, as a run through the Room is; its creator makes it visible.
    .with_creator(Some(agents[0].clone()));
    run.add_failure_event(WorkflowFailureEvent::new(
        WorkflowFailureKind::StepNeedsHelp,
        "node-run-2",
        vec![],
        "the deploy step needs a person",
    ));
    run.set_status(WorkflowRunStatus::Failed);
    session.create_workflow_run(run);
    app.sessions_mut().restore_session(session);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let mut attention = Vec::new();
    for agent_id in &agents {
        let request = LocalDaemonRequest::GetPaneFeed(GetPaneFeedRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
        let command = KernelCommand::from_local_request("pane", None, None, &request);
        let LocalDaemonResponse::PaneFeed { feed } =
            router.dispatch(command, request).await.unwrap()
        else {
            panic!("expected the pane feed");
        };
        attention.push(
            feed.stripes
                .into_iter()
                .filter(|stripe| stripe.id.starts_with("attention:"))
                .map(|stripe| stripe.text)
                .collect::<Vec<_>>(),
        );
    }
    assert!(
        attention[0].is_empty(),
        "the entry agent's pane: {:?}",
        attention[0]
    );
    assert!(
        attention[1].is_empty(),
        "the middle agent's pane: {:?}",
        attention[1]
    );
    assert_eq!(
        attention[2].len(),
        1,
        "the failing agent's pane: {:?}",
        attention[2]
    );
    assert!(
        attention[2][0].ends_with("failed: deploy needs help — the deploy step needs a person"),
        "{}",
        attention[2][0]
    );
}
