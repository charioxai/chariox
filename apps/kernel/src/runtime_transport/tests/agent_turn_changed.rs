//! MP-08 / MP-10 / MP-11: `agent_turn_changed` is the history outline's latest turn, as the kernel
//! answers it at that moment, for text, tool calls, blobs, cancel and failure.
use super::*;
use crate::history::OperationalHistoryStore;
use crate::history::{
    HistoryEvent, HistoryEventKind, HistoryEventRole, HistoryEventTurnContext, SessionHistoryEntry,
    PROMPT_SETTLED_AT_MS_METADATA_KEY, PROMPT_SETTLEMENT_STATUS_METADATA_KEY,
};
use crate::local::{
    GetSessionHistoryOutlineRequest, LocalDaemonResponse, SessionHistoryOutlineTurn,
    SessionHistoryOutlineTurnLifecycle,
};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::terminal::TerminalOutputKind;
use std::collections::BTreeMap;

struct Watched {
    app: Arc<Mutex<DaemonApp>>,
    history: OperationalHistoryStore,
    router: Arc<CommandRouter>,
    session_id: String,
    attachment_id: String,
    agent_id: String,
    events: mpsc::Receiver<KernelOutgoingFrame>,
    task: tokio::task::JoinHandle<()>,
}

impl Watched {
    async fn start() -> Self {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .unwrap()
            .0;
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "turn-subscriber",
                crate::attachment::ClientCapabilityLevel::MessageTransport,
            ))
            .unwrap();
        let agent_id = crate::app::KernelSessionReadService::new(&mut app)
            .session_snapshot(session.id())
            .unwrap()
            .agents()[0]
            .id()
            .to_string();
        let history = app.operational_history_store();
        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 4));
        let runtime = Arc::new(KernelTransportRuntime::new(router.transport_health_store()));
        let (priority_tx, _priority_rx) = mpsc::channel(64);
        let (event_tx, events) = mpsc::channel(256);
        let (close_tx, _close_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(super::super::subscriptions::run_subscription_loop(
            router.clone(),
            runtime,
            super::super::outgoing::KernelOutgoingSender::new(priority_tx, event_tx),
            close_tx,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            KernelSubscription {
                session_id: session.id().into(),
                attachment_id: attachment.id().into(),
                subscription_scope: KernelSubscriptionScope::Session,
                connection_class: crate::local::KernelConnectionClass::Terminal,
            },
        ));
        let mut watched = Self {
            app,
            history,
            router,
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            agent_id,
            events,
            task,
        };
        watched
            .next_event(|event| matches!(event, KernelEvent::SessionSnapshot { .. }))
            .await;
        watched
    }

    async fn next_event(&mut self, wanted: impl Fn(&KernelEvent) -> bool) -> KernelEvent {
        let mut seen = Vec::new();
        let found = timeout(Duration::from_secs(5), async {
            loop {
                if let KernelOutgoingFrame::Event { event, .. } =
                    self.events.recv().await.expect("subscription closed")
                {
                    if wanted(&event) {
                        return *event;
                    }
                    seen.push(crate::transport::kernel_protocol::kernel_event_name(&event));
                }
            }
        })
        .await;
        found.unwrap_or_else(|_| panic!("the expected event did not arrive; saw {seen:?}"))
    }

    fn context(&self, prompt_id: &str) -> HistoryEventTurnContext {
        HistoryEventTurnContext {
            session_id: Some(self.session_id.clone()),
            agent_id: Some(self.agent_id.clone()),
            turn_id: Some(prompt_id.into()),
            prompt_id: Some(prompt_id.into()),
            provider_run_id: Some("run-1".into()),
            ..HistoryEventTurnContext::default()
        }
    }

    /// Persist rows, then wake the subscription the way provider output does.
    async fn append(&self, events: Vec<HistoryEvent>) {
        self.history.append_many(&events).unwrap();
        let mut app = self.app.lock().await;
        app.terminal_mut().fan_out_output(
            &self.session_id,
            "run-1",
            Some(&self.agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("wake".into()),
            vec![self.attachment_id.clone()],
            b"wake",
        );
    }

    fn sequence(&self) -> u64 {
        // The same counter the kernel's own history rows use.
        self.history.reserve_sequence()
    }

    fn prompt(&self, prompt_id: &str, text: &str) -> HistoryEvent {
        HistoryEvent::transcript(
            self.sequence(),
            &SessionHistoryEntry::user_prompt(
                &self.session_id,
                "attachment-1",
                &self.agent_id,
                text,
            ),
            self.context(prompt_id),
        )
    }

    fn output(
        &self,
        prompt_id: &str,
        kind: TerminalOutputKind,
        key: &str,
        text: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            self.sequence(),
            &SessionHistoryEntry::provider_output(
                &self.session_id,
                "run-1",
                Some(&self.agent_id),
                kind,
                Some(key.into()),
                text.to_string(),
            ),
            self.context(prompt_id),
        )
    }

    fn settlement(&self, prompt_id: &str, status: &str) -> HistoryEvent {
        HistoryEvent::operational(
            self.sequence(),
            HistoryEventKind::ProviderStatus,
            Some(HistoryEventRole::System),
            None,
            BTreeMap::from([
                (
                    PROMPT_SETTLED_AT_MS_METADATA_KEY.into(),
                    serde_json::json!(crate::session::unix_epoch_ms() + 1),
                ),
                (
                    PROMPT_SETTLEMENT_STATUS_METADATA_KEY.into(),
                    serde_json::json!(status),
                ),
            ]),
            self.context(prompt_id),
        )
    }

    /// The latest turn as `GetSessionHistoryOutline` answers it now.
    async fn outline_turn(&self) -> SessionHistoryOutlineTurn {
        let request =
            LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
                session_id: self.session_id.clone(),
                agent_ids: Some(vec![self.agent_id.clone()]),
                latest_prompt_count: Some(1),
                cursor: None,
                include_prefetched_content: false,
            });
        let command = KernelCommand::from_local_request_with_caller(
            "outline-oracle",
            KernelCommandSource::LocalCli,
            KernelCaller::for_source(&KernelCommandSource::LocalCli)
                .with_connection_class(KernelConnectionClass::Terminal),
            None,
            None,
            &request,
        );
        let LocalDaemonResponse::SessionHistoryOutline { agents } =
            self.router.dispatch(command, request).await.unwrap()
        else {
            panic!("expected a history outline");
        };
        agents[0].turns.last().cloned().expect("a turn")
    }

    /// The next `agent_turn_changed`, which must be the outline turn right now.
    async fn expect_turn_equal_to_outline(&mut self) -> SessionHistoryOutlineTurn {
        let KernelEvent::AgentTurnChanged {
            session_id,
            agent_id,
            history_revision,
            turn,
        } = self
            .next_event(|event| matches!(event, KernelEvent::AgentTurnChanged { .. }))
            .await
        else {
            unreachable!()
        };
        assert_eq!(
            (session_id, agent_id),
            (self.session_id.clone(), self.agent_id.clone())
        );
        assert!(history_revision > 0);
        assert_eq!(*turn, self.outline_turn().await);
        *turn
    }

    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
        self.router
            .runtime_state()
            .shutdown_cleanup()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn turn_event_follows_text_and_ends_with_the_final_status() {
    let mut watched = Watched::start().await;
    watched
        .append(vec![
            watched.prompt("prompt-1", "say hi"),
            watched.output(
                "prompt-1",
                TerminalOutputKind::ProviderOutput,
                "text-1",
                "Hel",
            ),
        ])
        .await;
    let open = watched.expect_turn_equal_to_outline().await;
    assert_eq!(open.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    watched
        .append(vec![watched.output(
            "prompt-1",
            TerminalOutputKind::ProviderOutput,
            "text-1",
            "lo",
        )])
        .await;
    watched.expect_turn_equal_to_outline().await;
    watched
        .append(vec![watched.settlement("prompt-1", "completed")])
        .await;
    let done = watched.expect_turn_equal_to_outline().await;
    assert_eq!(
        done.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    watched.stop().await;
}

#[tokio::test]
async fn turn_event_carries_tool_calls_and_blobs() {
    let mut watched = Watched::start().await;
    let mut rows = vec![watched.prompt("prompt-1", "read it")];
    for call in 0..30 {
        rows.push(watched.output(
            "prompt-1",
            TerminalOutputKind::ProviderTool,
            &format!("call-{call}"),
            &format!(r#"{{"id":"call-{call}","tool":"read","status":"completed","input":{{"filePath":"f{call}"}}}}"#),
        ));
    }
    rows.push(watched.output(
        "prompt-1",
        TerminalOutputKind::ProviderOutput,
        "text-1",
        "Done.",
    ));
    watched.append(rows).await;
    let turn = watched.expect_turn_equal_to_outline().await;
    assert!(!turn.blobs.is_empty(), "tool calls must show as blobs");
    watched.stop().await;
}

#[tokio::test]
async fn turn_event_ends_a_cancelled_and_a_failed_turn() {
    for (status, lifecycle) in [
        ("cancelled", SessionHistoryOutlineTurnLifecycle::Cancelled),
        ("failed", SessionHistoryOutlineTurnLifecycle::Failed),
    ] {
        let mut watched = Watched::start().await;
        watched
            .append(vec![
                watched.prompt("prompt-1", "go"),
                watched.output(
                    "prompt-1",
                    TerminalOutputKind::ProviderOutput,
                    "text-1",
                    "Wor",
                ),
            ])
            .await;
        watched.expect_turn_equal_to_outline().await;
        watched
            .append(vec![watched.settlement("prompt-1", status)])
            .await;
        let ended = watched.expect_turn_equal_to_outline().await;
        assert_eq!(ended.lifecycle, lifecycle);
        watched.stop().await;
    }
}

#[tokio::test]
async fn turn_events_are_coalesced_to_one_per_window() {
    let mut watched = Watched::start().await;
    watched
        .append(vec![watched.prompt("prompt-1", "stream")])
        .await;
    watched.expect_turn_equal_to_outline().await;
    let started = std::time::Instant::now();
    for chunk in 0..20 {
        watched
            .append(vec![watched.output(
                "prompt-1",
                TerminalOutputKind::ProviderOutput,
                "text-1",
                &format!("{chunk} "),
            )])
            .await;
    }
    let mut seen = 0;
    let mut last = None;
    while let Ok(event) = timeout(Duration::from_millis(400), async {
        watched
            .next_event(|event| matches!(event, KernelEvent::AgentTurnChanged { .. }))
            .await
    })
    .await
    {
        seen += 1;
        last = Some(event);
    }
    let windows = started.elapsed().as_millis() / 50 + 1;
    assert!(
        seen >= 1 && (seen as u128) <= windows,
        "{seen} events in {windows} windows"
    );
    let Some(KernelEvent::AgentTurnChanged { turn, .. }) = last else {
        panic!("a burst must end with its latest turn");
    };
    assert_eq!(*turn, watched.outline_turn().await);
    watched.stop().await;
}
