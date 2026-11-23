//! F4: Claude's folder-trust question on a native terminal is one card for every client of the
//! Room; an answer from any client is typed into Claude, and an answer in the terminal ends it.
use super::*;

const DIALOG: &str = r#"printf '%s\n' 'Accessing workspace:' '/work/space' 'Quick safety check: Is this a project you trust?' 'Do you trust this folder?' '❯ No, exit' '  Yes, I trust this folder'"#;
const COMPOSER: &str =
    r#"printf '%s\n' '❯ Try "write a test for foo"' '  ⏵⏵ manual mode on (shift+tab to cycle)'"#;

struct Room {
    _worktree: crate::test_support::TestWorktree,
    app: Arc<Mutex<DaemonApp>>,
    runtime: KernelRuntimeState,
    session: String,
    run: String,
    card: String,
    capture: std::path::PathBuf,
}

impl Room {
    /// A native Claude terminal whose program is `script`, with its output pumped by the runtime.
    async fn open(label: &str, script: &str) -> Self {
        let worktree = crate::test_support::TestWorktree::new(label);
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .expect("session should be created");
        let context_file = worktree.path().join("hidden-context.txt");
        let events_file = worktree.path().join("events.jsonl");
        let capture = worktree.path().join("typed.log");
        std::fs::write(&context_file, "").unwrap();
        std::fs::write(&events_file, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "sonnet",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut run = crate::provider::RuntimeProviderRun::new(
            format!("{label}-run"),
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: label.to_string(),
                pty_target: Some(label.to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec![
                    "-lc".to_string(),
                    script.replace("CAPTURE", &capture.display().to_string()),
                ],
                pty_env: std::collections::BTreeMap::from([
                    (
                        "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                        context_file.display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                        events_file.display().to_string(),
                    ),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.pty_mut()
            .spawn_for_run(&run)
            .expect("test provider PTY should start");
        app.providers_mut().insert_run_for_test(run.clone());
        crate::app::ProviderProcessTracker::new(&mut app)
            .register_managed_run(&run)
            .expect("native fixture must register its managed process");
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        Self {
            _worktree: worktree,
            app,
            runtime,
            session: session.id().to_string(),
            card: format!("claude-native-trust-{}", run.id()),
            run: run.id().to_string(),
            capture,
        }
    }

    async fn pump(&self) {
        self.runtime
            .pump_owned_provider_output(&self.session, &self.run, Vec::new(), false)
            .await
            .expect("the trust dialog is ordinary terminal output");
    }

    fn cards(&self) -> Vec<String> {
        self.runtime
            .owned
            .session_store
            .get_session(&self.session)
            .unwrap()
            .active_interactions()
            .iter()
            .map(|card| card.id().to_string())
            .collect()
    }

    /// Pumps until the room shows exactly `want` cards.
    async fn until_cards(&self, want: &[&str]) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            self.pump().await;
            if self.cards() == want {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("cards {:?}, wanted {want:?}", self.cards());
    }

    async fn answer(&self, choice: &str, client: &str) {
        use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity(self.app.clone(), 8);
        let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
            .with_connection_class(crate::local::KernelConnectionClass::Terminal);
        caller.caller_id = client.into();
        caller.client_id = Some(client.into());
        caller.user_id = Some("local".into());
        let request = crate::local::LocalDaemonRequest::RespondToInteraction(
            crate::local::RespondToInteractionRequest {
                session_id: self.session.clone(),
                interaction_id: self.card.clone(),
                choice_id: choice.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            },
        );
        let command = KernelCommand::from_local_request_with_caller(
            "trust-answer",
            KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        router
            .dispatch(command, request)
            .await
            .expect("the card accepts its answer");
    }

    async fn close(self) {
        self.runtime
            .with_app_side_effect(|app| app.pty_mut().remove_process(&self.run))
            .await
            .expect("test provider PTY should stop");
    }
}

#[tokio::test]
async fn the_trust_question_is_one_card_any_client_can_answer() {
    let room = Room::open("trust-card-answer", &format!("{DIALOG}; cat > CAPTURE")).await;
    let card = room.card.clone();
    room.until_cards(&[card.as_str()]).await;
    // Pumping again must not raise it twice, and a client that joins now still sees it.
    room.pump().await;
    room.pump().await;
    assert_eq!(
        room.cards(),
        vec![card.clone()],
        "one card, not one per drain"
    );

    room.answer("allow_once", "second-client").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let typed = loop {
        let typed = std::fs::read(&room.capture).unwrap_or_default();
        if typed.contains(&b'\n') || std::time::Instant::now() > deadline {
            break typed;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert_eq!(
        typed, b"\x1b[B\n",
        "the highlighted No moves down to Yes, then Enter confirms: {typed:?}"
    );
    room.close().await;
}

#[tokio::test]
async fn answering_in_the_terminal_withdraws_the_card() {
    let room = Room::open(
        "trust-card-local",
        &format!("{DIALOG}; sleep 1; {COMPOSER}; sleep 5"),
    )
    .await;
    let card = room.card.clone();
    room.until_cards(&[card.as_str()]).await;
    room.until_cards(&[]).await;
    room.close().await;
}
