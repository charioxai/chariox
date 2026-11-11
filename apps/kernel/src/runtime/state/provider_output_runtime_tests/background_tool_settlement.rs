use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;

// MP-08 / MP-10: the snapshots the Claude and Codex adapters emit for a
// background command (their tests pin them to this protocol fixture).
pub(super) fn lifecycle(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../local/api/tests/protocol_shapes/provider-tool-background-lifecycle.json"
    ))
    .unwrap();
    fixture[name].clone()
}

pub(super) fn tool_chunk(payload: &serde_json::Value) -> ProviderPromptChunk {
    ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: payload["id"].as_str().map(str::to_string),
        bytes: serde_json::to_vec(payload).unwrap(),
    }
}

#[derive(Clone, Copy)]
enum Submitter {
    Person,
    /// A one-node workflow of the agent, named by the given label.
    Workflow(&'static str),
}

impl Submitter {
    /// Starts a prompt on the idle agent; its workflow id when a workflow sent it.
    fn submit(
        self,
        app: &mut DaemonApp,
        session: &str,
        attachment: &str,
        agent: &str,
        text: &str,
    ) -> Option<String> {
        let Submitter::Workflow(name) = self else {
            app.submit_prompt(session, attachment, Some(agent), text, Vec::new())
                .unwrap();
            return None;
        };
        let workflow = app
            .sessions_mut()
            .create_workflow(session, Some(name.to_string()))
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(session, workflow.id(), agent)
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(session, workflow.id(), node.id(), Some(name.to_string()))
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session,
                workflow.id(),
                endpoint.id(),
                Some(text.to_string()),
            )
            .unwrap();
        let node_run_id = run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .start_workflow_node_run(session, run.id(), &node_run_id)
            .unwrap();
        let prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
            agent,
            text.to_string(),
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(run.id(), &node_run_id);
        let crate::session::PromptSubmissionOutcome::Started { .. } = app
            .prompt_owner_submit_prepared_prompt(session, prompt, false)
            .unwrap()
        else {
            panic!("the workflow prompt starts on the idle agent");
        };
        Some(workflow.id().to_string())
    }
}

/// How a provider's tool snapshots reach the kernel.
#[derive(Clone, Copy)]
enum OutputPath {
    /// A structured run's adapter batch.
    Structured,
    /// The native Claude bridge's transcript drain, one row at a time.
    NativeTranscript,
}

struct BackgroundTurn {
    runtime: KernelRuntimeState,
    session: String,
    agent: String,
    attachment: String,
    run: String,
    path: OutputPath,
}

impl BackgroundTurn {
    /// A turn whose agent launched one Claude and one Codex background command
    /// and finished one foreground call.
    async fn start() -> Self {
        Self::start_on("claude-code").await
    }

    /// The same turn on a provider whose output the kernel pumps itself.
    async fn start_on(provider: &str) -> Self {
        Self::start_with(provider, Submitter::Person, OutputPath::Structured).await
    }

    async fn start_with(provider: &str, submitter: Submitter, path: OutputPath) -> Self {
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
                    provider,
                    "default",
                    "test",
                )
                .with_agent_id(agent.id()),
            )
            .unwrap();
        submitter.submit(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
            "start the server in the background",
        );
        let app = Arc::new(Mutex::new(app));
        let turn = Self {
            runtime: owned_runtime_state(&app).await,
            session: session.id().to_string(),
            agent: agent.id().to_string(),
            attachment: attachment.id().to_string(),
            run: run.id().to_string(),
            path,
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

    /// The merge key the call's rows are stored under on this turn's path.
    fn merge_key(&self, tool_id: &str) -> String {
        match self.path {
            OutputPath::Structured => tool_id.to_string(),
            OutputPath::NativeTranscript => {
                format!("claude-transcript:{}:tool:{tool_id}", self.run)
            }
        }
    }

    async fn apply(&self, chunks: Vec<ProviderPromptChunk>) {
        if let OutputPath::NativeTranscript = self.path {
            let mut app = self.runtime.app.lock().await;
            for chunk in chunks {
                app.fan_out_output(
                    &self.session,
                    &self.run,
                    chunk.kind,
                    chunk.merge_key.map(|id| self.merge_key(&id)),
                    vec![self.attachment.clone()],
                    &chunk.bytes,
                );
            }
            return;
        }
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

impl BackgroundTurn {
    /// The turn settled and its provider process died before any task
    /// notification arrived, with no prompt left to fail.
    async fn settled_turn_with_dead_provider(&self) {
        self.live_tools();
        self.runtime
            .owned
            .complete_local_prompt_without_advance(&self.session, &self.agent, Some(&self.run))
            .unwrap();
        self.runtime
            .owned
            .provider_store
            .terminate_run_provider_only(&self.session, &self.run)
            .unwrap();
    }

    async fn assert_background_commands_ended_at_or_after(&self, before: u64) {
        let ended = self.live_tools();
        let mut ids = ended
            .iter()
            .map(|tool| tool["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["call_sleep", "toolu_bg"], "{ended:?}");
        for tool in &ended {
            assert!(tool["finished_at_ms"].as_u64().unwrap() >= before, "{tool}");
        }
        let stored = self.stored_tools().await;
        assert!(stored["toolu_bg"]["finished_at_ms"].is_u64());
        assert!(stored["call_sleep"]["finished_at_ms"].is_u64());
    }
}

#[tokio::test]
async fn liveness_seeing_a_dead_provider_without_an_active_prompt_ends_its_background_commands() {
    let turn = BackgroundTurn::start().await;
    turn.settled_turn_with_dead_provider().await;
    let before = crate::session::unix_epoch_ms();
    turn.runtime
        .reconcile_provider_run_exit(&turn.session, &turn.run)
        .await
        .unwrap();
    turn.assert_background_commands_ended_at_or_after(before)
        .await;
}

#[tokio::test]
async fn the_output_pump_seeing_a_dead_provider_without_an_active_prompt_ends_its_background_commands(
) {
    let turn = BackgroundTurn::start_on("slow-structured").await;
    turn.settled_turn_with_dead_provider().await;
    let before = crate::session::unix_epoch_ms();
    turn.runtime
        .pump_owned_provider_output(
            &turn.session,
            &turn.run,
            vec![turn.attachment.clone()],
            false,
        )
        .await
        .unwrap();
    turn.assert_background_commands_ended_at_or_after(before)
        .await;
}

impl BackgroundTurn {
    /// The prompt each stored row of a tool call belongs to, oldest row first.
    async fn stored_prompts_of(&self, tool_id: &str) -> Vec<Option<String>> {
        self.runtime
            .flush_provider_history(&self.run)
            .await
            .unwrap();
        self.runtime
            .owned
            .operational_history_store
            .load_session_events_for_agent_sequence_range(
                &self.session,
                &self.agent,
                0,
                i64::MAX as u64,
            )
            .unwrap()
            .into_iter()
            .filter(|event| {
                event
                    .to_session_history_entry()
                    .is_some_and(|entry| entry.merge_key.as_deref() == Some(tool_id))
            })
            .map(|event| event.prompt_id)
            .collect()
    }
}

#[tokio::test]
async fn a_background_finish_under_a_newer_prompt_stays_in_the_turn_that_started_it() {
    let turn = BackgroundTurn::start().await;
    turn.live_tools();
    let launch_prompt = turn.stored_prompts_of("toolu_bg").await[0]
        .clone()
        .expect("the launch row belongs to its prompt");
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
    turn.apply(vec![
        tool_chunk(&lifecycle("claude_background_finished")),
        tool_chunk(&lifecycle("codex_background_finished")),
    ])
    .await;

    // Live: the finish reaches clients as part of the launching prompt.
    let live = turn
        .runtime
        .owned
        .terminal_stream
        .drain_output_records(&turn.session, &turn.attachment)
        .into_iter()
        .filter(|record| record.kind == TerminalOutputKind::ProviderTool)
        .map(|record| {
            let tool: serde_json::Value = serde_json::from_slice(&record.bytes).unwrap();
            (tool["id"].as_str().unwrap().to_string(), record.prompt_id)
        })
        .collect::<Vec<_>>();
    for (id, prompt) in &live {
        if id == "toolu_newer" {
            assert_ne!(prompt.as_deref(), Some(launch_prompt.as_str()), "{live:?}");
        } else {
            assert_eq!(prompt.as_deref(), Some(launch_prompt.as_str()), "{live:?}");
        }
    }
    // In history: the finish took the place of each call's newest row, so
    // every row of the two background calls is still in that turn.
    for (id, rows) in [("toolu_bg", 2), ("call_sleep", 1)] {
        let prompts = turn.stored_prompts_of(id).await;
        assert_eq!(prompts.len(), rows, "{id}: {prompts:?}");
        assert!(
            prompts
                .iter()
                .all(|prompt| prompt.as_deref() == Some(launch_prompt.as_str())),
            "{id}: {prompts:?}"
        );
    }
    let stored = turn.stored_tools().await;
    assert!(stored["toolu_bg"]["finished_at_ms"].is_u64());
    assert!(stored["call_sleep"]["finished_at_ms"].is_u64());
}

impl BackgroundTurn {
    /// The stored rows of a tool call, oldest first.
    async fn stored_events_of(&self, tool_id: &str) -> Vec<crate::history::HistoryEvent> {
        let merge_key = self.merge_key(tool_id);
        self.runtime
            .flush_provider_history(&self.run)
            .await
            .unwrap();
        self.runtime
            .owned
            .operational_history_store
            .load_session_events_for_agent_sequence_range(
                &self.session,
                &self.agent,
                0,
                i64::MAX as u64,
            )
            .unwrap()
            .into_iter()
            .filter(|event| {
                event
                    .to_session_history_entry()
                    .is_some_and(|entry| entry.merge_key.as_deref() == Some(merge_key.as_str()))
            })
            .collect()
    }

    /// The launch settles; `later` sends the next prompt, under which the
    /// background command ends.
    async fn finish_under(&self, later: Submitter) -> Option<String> {
        self.runtime
            .owned
            .complete_local_prompt_without_advance(&self.session, &self.agent, Some(&self.run))
            .unwrap();
        let later_workflow = later.submit(
            &mut *self.runtime.app.lock().await,
            &self.session,
            &self.attachment,
            &self.agent,
            "and now something else",
        );
        self.apply(vec![tool_chunk(&lifecycle("claude_background_finished"))])
            .await;
        later_workflow
    }
}

/// A late finish keeps every attribution of the row it replaces: its turn,
/// prompt, workflow, and the prompt origin and source live records carry.
async fn a_late_finish_keeps_the_provenance_of_its_launch(
    launch: Submitter,
    later: Submitter,
    path: OutputPath,
) {
    let turn = BackgroundTurn::start_with("claude-code", launch, path).await;
    turn.live_tools();
    let launch_row = turn.stored_events_of("toolu_bg").await.pop().unwrap();
    let launch_workflow = launch_row.workflow_id.clone();
    assert_eq!(
        launch_workflow.is_some(),
        matches!(launch, Submitter::Workflow(_))
    );
    let later_workflow = turn.finish_under(later).await;

    let live = turn
        .runtime
        .owned
        .terminal_stream
        .drain_output_records(&turn.session, &turn.attachment)
        .into_iter()
        .filter(|record| record.kind == TerminalOutputKind::ProviderTool)
        .collect::<Vec<_>>();
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(live[0].prompt_id, launch_row.prompt_id);
    assert_eq!(
        live[0]
            .prompt_origin
            .map(|origin| serde_json::to_value(origin).unwrap()),
        launch_row.metadata.get("prompt_origin").cloned()
    );
    assert_eq!(
        live[0].source_attachment_id.as_deref(),
        launch_row
            .metadata
            .get("source_attachment_id")
            .and_then(serde_json::Value::as_str)
    );

    let rows = turn.stored_events_of("toolu_bg").await;
    let finished = rows.last().unwrap();
    assert!(finished
        .content
        .as_deref()
        .is_some_and(|content| content.contains("finished_at_ms\":1")));
    assert_eq!(finished.sequence, launch_row.sequence);
    for (field, kept, launched) in [
        ("turn_id", &finished.turn_id, &launch_row.turn_id),
        ("prompt_id", &finished.prompt_id, &launch_row.prompt_id),
        (
            "workflow_id",
            &finished.workflow_id,
            &launch_row.workflow_id,
        ),
        (
            "workflow_run_id",
            &finished.workflow_run_id,
            &launch_row.workflow_run_id,
        ),
        (
            "workflow_node_id",
            &finished.workflow_node_id,
            &launch_row.workflow_node_id,
        ),
    ] {
        assert_eq!(kept, launched, "{field}");
    }
    for key in ["prompt_origin", "source_attachment_id"] {
        assert_eq!(
            finished.metadata.get(key),
            launch_row.metadata.get(key),
            "{key}"
        );
    }

    for workflow in launch_workflow.iter().chain(later_workflow.iter()) {
        let shown = turn
            .runtime
            .owned
            .operational_history_store
            .query_events(crate::history::HistoryEventQuery {
                session_id: Some(turn.session.clone()),
                workflow_id: Some(workflow.clone()),
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .any(|event| event.event_id == launch_row.event_id);
        assert_eq!(
            shown,
            Some(workflow) == launch_workflow.as_ref(),
            "workflow {workflow}"
        );
    }
}

#[tokio::test]
async fn a_workflow_launched_background_finish_under_a_person_prompt_stays_with_the_workflow() {
    a_late_finish_keeps_the_provenance_of_its_launch(
        Submitter::Workflow("workflow-a"),
        Submitter::Person,
        OutputPath::Structured,
    )
    .await;
}

#[tokio::test]
async fn a_workflow_launched_background_finish_under_another_workflow_stays_with_its_own() {
    a_late_finish_keeps_the_provenance_of_its_launch(
        Submitter::Workflow("workflow-a"),
        Submitter::Workflow("workflow-b"),
        OutputPath::Structured,
    )
    .await;
}

#[tokio::test]
async fn a_person_launched_background_finish_under_a_workflow_stays_out_of_that_workflow() {
    a_late_finish_keeps_the_provenance_of_its_launch(
        Submitter::Person,
        Submitter::Workflow("workflow-b"),
        OutputPath::Structured,
    )
    .await;
}

#[tokio::test]
async fn a_native_claude_background_finish_under_another_workflow_stays_with_its_own() {
    a_late_finish_keeps_the_provenance_of_its_launch(
        Submitter::Workflow("workflow-a"),
        Submitter::Workflow("workflow-b"),
        OutputPath::NativeTranscript,
    )
    .await;
}

#[tokio::test]
async fn a_native_claude_background_finish_under_a_workflow_stays_in_its_person_turn() {
    a_late_finish_keeps_the_provenance_of_its_launch(
        Submitter::Person,
        Submitter::Workflow("workflow-b"),
        OutputPath::NativeTranscript,
    )
    .await;
}
