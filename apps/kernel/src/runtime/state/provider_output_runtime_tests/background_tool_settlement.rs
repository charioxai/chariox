use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;

// MP-08 / MP-10: the snapshots the Claude and Codex adapters emit for a
// background command (their tests pin them to this protocol fixture).
fn lifecycle(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../local/api/tests/protocol_shapes/provider-tool-background-lifecycle.json"
    ))
    .unwrap();
    fixture[name].clone()
}

fn tool_chunk(payload: &serde_json::Value) -> ProviderPromptChunk {
    ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: payload["id"].as_str().map(str::to_string),
        bytes: serde_json::to_vec(payload).unwrap(),
    }
}

struct BackgroundTurn {
    runtime: KernelRuntimeState,
    session: String,
    agent: String,
    attachment: String,
    run: String,
}

impl BackgroundTurn {
    /// A turn whose agent launched one Claude and one Codex background command
    /// and finished one foreground call.
    async fn start() -> Self {
        let worktree = crate::test_support::TestWorktree::new("background-tool-settlement");
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "background-tool-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "test",
                )
                .with_agent_id(agent.id()),
            )
            .unwrap();
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "start the server in the background",
            Vec::new(),
        )
        .unwrap();
        let app = Arc::new(Mutex::new(app));
        let turn = Self {
            runtime: owned_runtime_state(&app).await,
            session: session.id().to_string(),
            agent: agent.id().to_string(),
            attachment: attachment.id().to_string(),
            run: run.id().to_string(),
        };
        let mut launching = lifecycle("claude_background_launched");
        launching["status"] = serde_json::json!("running");
        launching.as_object_mut().unwrap().remove("output");
        turn.apply(vec![
            tool_chunk(&launching),
            tool_chunk(&lifecycle("claude_background_launched")),
            tool_chunk(&lifecycle("codex_background_running")),
            tool_chunk(&lifecycle("claude_foreground_completed")),
        ])
        .await;
        turn
    }

    async fn apply(&self, chunks: Vec<ProviderPromptChunk>) {
        self.runtime
            .apply_owned_structured_output_batch(
                &self.session,
                &self.run,
                vec![self.attachment.clone()],
                ProviderPromptSignalBatch {
                    chunks,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    /// The tool snapshots clients received since the last call.
    fn live_tools(&self) -> Vec<serde_json::Value> {
        self.runtime
            .owned
            .terminal_stream
            .drain_output_records(&self.session, &self.attachment)
            .into_iter()
            .filter(|record| record.kind == TerminalOutputKind::ProviderTool)
            .map(|record| serde_json::from_slice(&record.bytes).unwrap())
            .collect()
    }

    /// Each call's snapshot as a client rebuilds it from history: the latest
    /// row of its merge key.
    async fn stored_tools(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.runtime
            .flush_provider_history(&self.run)
            .await
            .unwrap();
        self.runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&self.session, Some(&self.agent))
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::ProviderTool)
            .map(|entry| {
                (
                    entry.merge_key.unwrap(),
                    serde_json::from_str(&entry.text).unwrap(),
                )
            })
            .collect()
    }
}

#[tokio::test]
async fn background_command_settles_after_its_turn_and_a_newer_one_live_and_in_history() {
    let turn = BackgroundTurn::start().await;
    turn.live_tools();
    let stored = turn.stored_tools().await;
    assert!(stored["toolu_bg"]["finished_at_ms"].is_null());
    assert_eq!(stored["toolu_bg"]["background"], true);
    assert_eq!(stored["toolu_fg"]["started_at_ms"], 1_793_700_000_000_u64);

    // The turn settles; a newer turn starts and runs its own call.
    turn.runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session, &turn.agent, Some(&turn.run))
        .unwrap();
    turn.runtime
        .app
        .lock()
        .await
        .submit_prompt(
            &turn.session,
            &turn.attachment,
            Some(&turn.agent),
            "and now something else",
            Vec::new(),
        )
        .unwrap();
    let mut newer = lifecycle("claude_foreground_completed");
    newer["id"] = serde_json::json!("toolu_newer");
    turn.apply(vec![tool_chunk(&newer)]).await;
    assert!(turn.stored_tools().await["toolu_bg"]["finished_at_ms"].is_null());

    // The older turn's commands end while the newer turn runs.
    turn.apply(vec![
        tool_chunk(&lifecycle("claude_background_finished")),
        tool_chunk(&lifecycle("codex_background_finished")),
    ])
    .await;
    let live = turn.live_tools();
    let live_finished = live
        .iter()
        .filter(|tool| tool["finished_at_ms"].is_u64())
        .map(|tool| tool["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(live_finished, ["toolu_bg", "call_sleep"]);
    let stored = turn.stored_tools().await;
    assert_eq!(stored["toolu_bg"], lifecycle("claude_background_finished"));
    assert_eq!(stored["call_sleep"], lifecycle("codex_background_finished"));
    assert!(stored["toolu_newer"].get("background").is_none());
}

#[tokio::test]
async fn a_lost_provider_session_ends_its_background_commands_once() {
    let turn = BackgroundTurn::start().await;
    turn.apply(vec![tool_chunk(&{
        let mut done = lifecycle("claude_background_finished");
        done["id"] = serde_json::json!("toolu_done");
        done
    })])
    .await;
    turn.live_tools();
    turn.runtime
        .flush_provider_history(&turn.run)
        .await
        .unwrap();
    let before = crate::session::unix_epoch_ms();
    turn.runtime
        .retire_owned_provider_run_after_terminal_failure(&turn.session, &turn.run)
        .await;
    let ended = turn.live_tools();
    let ids = ended
        .iter()
        .map(|tool| tool["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2, "{ended:?}");
    for tool in &ended {
        assert_eq!(tool["background"], true);
        assert!(tool["finished_at_ms"].as_u64().unwrap() >= before, "{tool}");
    }
    let claude = ended.iter().find(|tool| tool["id"] == "toolu_bg").unwrap();
    assert_eq!(claude["status"], "completed");
    let codex = ended
        .iter()
        .find(|tool| tool["id"] == "call_sleep")
        .unwrap();
    assert_eq!(codex["status"], "cancelled");
    let stored = turn.stored_tools().await;
    assert_eq!(
        stored["toolu_bg"]["finished_at_ms"],
        claude["finished_at_ms"]
    );
    assert_eq!(stored["toolu_done"], {
        let mut done = lifecycle("claude_background_finished");
        done["id"] = serde_json::json!("toolu_done");
        done
    });
    // A later settlement finds nothing left running.
    turn.runtime
        .owned
        .end_background_provider_tools(&turn.session, &turn.run, &turn.agent)
        .unwrap();
    assert!(turn.live_tools().is_empty());
}

#[tokio::test]
async fn a_kernel_restart_ends_background_commands_of_runs_it_did_not_take_over() {
    let turn = BackgroundTurn::start().await;
    turn.live_tools();
    turn.runtime
        .flush_provider_history(&turn.run)
        .await
        .unwrap();
    // The run is still live: its commands keep running.
    turn.runtime.end_background_provider_tools_left_by_restart();
    assert!(turn.live_tools().is_empty());
    // After a cold restart no process of the run is left.
    turn.runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&turn.session, &turn.run)
        .unwrap();
    turn.runtime.end_background_provider_tools_left_by_restart();
    let ended = turn.live_tools();
    let mut ids = ended
        .iter()
        .map(|tool| tool["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, ["call_sleep", "toolu_bg"]);
    let stored = turn.stored_tools().await;
    assert!(stored["toolu_bg"]["finished_at_ms"].is_u64());
    assert!(stored["call_sleep"]["finished_at_ms"].is_u64());
    assert!(stored["toolu_fg"].get("finished_at_ms").is_none());
}
