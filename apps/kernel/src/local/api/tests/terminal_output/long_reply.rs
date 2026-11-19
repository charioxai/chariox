use super::*;

struct LongReplyFixture {
    harness: LocalRouterTestHarness,
    config: DaemonConfig,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    run_id: String,
    _worktree: crate::test_support::TestWorktree,
}

impl LongReplyFixture {
    fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("long-reply");
        let config = DaemonConfig::for_tests();
        let harness = LocalRouterTestHarness::with_config(config.clone());
        let LocalDaemonResponse::SessionCreated { session, agent, .. } = harness
            .dispatch(LocalDaemonRequest::CreateSession(
                worktree.session_request(),
            ))
            .unwrap()
        else {
            panic!("session should create")
        };
        let LocalDaemonResponse::SessionAttached { attachment } = harness
            .dispatch(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().into(),
                    client_id: "long-reply".into(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .unwrap()
        else {
            panic!("client should attach")
        };
        let run_id = harness.with_app_mut(|app| {
            app.launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "slow-structured",
                    "default",
                    "default",
                )
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui),
            )
            .unwrap()
            .id()
            .to_string()
        });
        Self {
            harness,
            config,
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            attachment_id: attachment.id().into(),
            run_id,
            _worktree: worktree,
        }
    }

    fn append(&self, kind: TerminalOutputKind, text: &str) -> Vec<u8> {
        let LocalDaemonResponse::TerminalOutput { records } = self
            .harness
            .dispatch(LocalDaemonRequest::AppendNativeProviderOutput(
                AppendNativeProviderOutputRequest {
                    session_id: self.session_id.clone(),
                    attachment_id: self.attachment_id.clone(),
                    provider_run_id: self.run_id.clone(),
                    kind,
                    merge_key: Some("long-reply".into()),
                    text: text.into(),
                },
            ))
            .unwrap()
        else {
            panic!("output should append")
        };
        assert_eq!(records.len(), 1);
        records.into_iter().next().unwrap().bytes
    }

    fn history(&self, text: Option<&str>) -> Vec<crate::history::HistoryEvent> {
        let LocalDaemonResponse::RecallEvents { events, .. } = self
            .harness
            .dispatch(LocalDaemonRequest::QueryRecall(QueryRecallRequest {
                session_id: Some(self.session_id.clone()),
                agent_id: Some(self.agent_id.clone()),
                text: text.map(str::to_owned),
                ..Default::default()
            }))
            .unwrap()
        else {
            panic!("history should load")
        };
        events
    }
}

fn long_reply_text() -> String {
    format!(
        "{}\nThe final sentence contains longreplytailword.\n",
        "正文🙂".repeat(4096)
    )
}

#[test]
fn long_reply_native_assistant_is_complete_live() {
    let fixture = LongReplyFixture::new();
    let text = long_reply_text();
    let live = fixture.append(TerminalOutputKind::ProviderOutput, &text);
    assert_eq!(live.len(), text.len(), "live reply must keep every byte");
    assert!(live == text.as_bytes());
}

#[test]
fn long_reply_native_reasoning_is_complete_live_and_in_history() {
    let fixture = LongReplyFixture::new();
    let text = long_reply_text();
    let live = fixture.append(TerminalOutputKind::ProviderReasoning, &text);
    assert_eq!(
        live.len(),
        text.len(),
        "live reasoning must keep every byte"
    );
    assert!(live == text.as_bytes());
    let events = fixture.history(None);
    assert!(events
        .iter()
        .any(|event| event.content.as_deref() == Some(text.as_str())));
}

#[test]
fn long_reply_native_assistant_survives_history_reload_and_kernel_restart() {
    let mut fixture = LongReplyFixture::new();
    let text = long_reply_text();
    fixture.append(TerminalOutputKind::ProviderOutput, &text);
    let complete = |events: Vec<crate::history::HistoryEvent>| {
        assert!(
            events
                .iter()
                .any(|event| event.content.as_deref() == Some(text.as_str())),
            "history must contain the whole reply"
        );
    };
    complete(fixture.history(None));
    drop(fixture.harness);
    fixture.harness = LocalRouterTestHarness::with_config(fixture.config.clone());
    complete(fixture.history(None));
}

#[test]
fn long_reply_native_assistant_tail_is_searchable() {
    let fixture = LongReplyFixture::new();
    fixture.append(TerminalOutputKind::ProviderOutput, &long_reply_text());
    assert_eq!(fixture.history(Some("longreplytailword")).len(), 1);
}

#[test]
fn long_reply_native_tool_stays_bounded_with_exact_omitted_bytes() {
    let fixture = LongReplyFixture::new();
    let text = long_reply_text();
    let live = fixture.append(TerminalOutputKind::ProviderTool, &text);
    assert!(live.len() <= 16 * 1024);
    let live = String::from_utf8(live).unwrap();
    let (kept, marker) = live
        .split_once("\n\n[chariox: output truncated, omitted ")
        .unwrap();
    assert!(text.starts_with(kept));
    let omitted: usize = marker.strip_suffix(" bytes]\n").unwrap().parse().unwrap();
    assert_eq!(omitted, text.len() - kept.len());
}
