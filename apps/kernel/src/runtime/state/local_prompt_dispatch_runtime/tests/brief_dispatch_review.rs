//! MP-08 / MP-10 / MP-11: supplementary review regressions at the actual final envelope seam.
use super::*;

const SECRET: &str = "registered-during-brief-test-value";
const BRIEF: &str = "## Goal\nRetain brief-only-amber\n## Constraints & Preferences\nNone\n## Progress\nDone\n## Key Decisions\nNone\n## Next Steps\nRecall\n## Critical Context\nbrief-only-amber";

struct PausedBrief {
    root: crate::test_support::TestWorktree,
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    prompt_id: String,
    run: crate::provider::RuntimeProviderRun,
}

impl PausedBrief {
    async fn new(register_before_capture: bool, stored_brief: bool) -> Self {
        let (root, runtime, session_id, agent_id, attachment_id, prompt_id, _) =
            runtime_with_active_prompt_and_history(Some(&format!(
                "historical value {SECRET} {}",
                "notes ".repeat(2_000)
            )))
            .await;
        if register_before_capture {
            runtime
                .owned
                .room_secret_observations
                .register(&session_id, SECRET)
                .unwrap();
        }
        if stored_brief {
            runtime
                .owned
                .operational_history_store
                .save_agent_handoff_brief(
                    &session_id,
                    &agent_id,
                    &crate::history::AgentHandoffBrief {
                        brief: BRIEF.into(),
                        covered_through_sequence: 1,
                    },
                )
                .unwrap();
        }
        Self::from_history(
            root,
            runtime,
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            "sonnet",
        )
    }
    fn from_history(
        root: crate::test_support::TestWorktree,
        runtime: KernelRuntimeState,
        session_id: String,
        agent_id: String,
        attachment_id: String,
        prompt_id: String,
        model: &str,
    ) -> Self {
        let started = root.path().join("brief-started");
        let release = root.path().join("brief-release");
        let captured = root.path().join("final-provider-envelope");
        let hidden_context = root.path().join("provider-hidden-context");
        std::fs::write(&hidden_context, "").unwrap();
        let utility_inputs = root.path().join("brief-utility-inputs");
        let answer = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":BRIEF}]}}).to_string();
        let script = format!(
            "#!/bin/sh\nwhile IFS= read -r line; do case \"$line\" in *'Update the handoff brief'*) printf '%s\\n' \"$line\" >> '{}'; touch '{}'; n=0; while [ ! -f '{}' ] && [ $n -lt 800 ]; do sleep .01; n=$((n+1)); done; printf '%s\\n' '{}' ;; *) printf '%s\\n' \"$line\" >> '{}' ;; esac; printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done\n",
            utility_inputs.display(), started.display(), release.display(), answer, captured.display()
        );
        let executable = root.path().join("paused-brief-fixture");
        std::fs::write(&executable, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let request = LaunchProviderRequest::new(&session_id, "claude", "claude", "default", model)
            .with_agent_id(&agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            "review-brief-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "supplementary-fixture".into(),
                pty_target: None,
                pty_program: Some(executable.display().to_string()),
                pty_args: vec![],
                pty_env: std::collections::BTreeMap::from([(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                    hidden_context.display().to_string(),
                )]),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime
            .owned
            .provider_store
            .initialize_runtime(&run)
            .unwrap();
        runtime.owned.provider_run_projection.update(run.clone());
        Self {
            root,
            runtime,
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            run,
        }
    }
    async fn prepare(&self) {
        self.prepare_with_inputs(String::new(), Vec::new()).await;
    }
    async fn prepare_with_inputs(
        &self,
        hidden: String,
        attachments: Vec<crate::session::PromptAttachment>,
    ) {
        let mut dispatch = dispatch(
            &self.session_id,
            &self.agent_id,
            &self.attachment_id,
            self.run.id(),
            &self.prompt_id,
            "cancelled request",
            None,
            false,
        );
        dispatch.hidden_system_context = hidden;
        dispatch.attachments = attachments;
        self.runtime
            .enqueue_prompt_dispatch(&dispatch)
            .await
            .unwrap();
        self.wait_for("brief-started").await;
    }
    async fn wait_for(&self, name: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !self.root.path().join(name).exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the actual provider fixture seam must be reached");
    }
    fn release(&self) {
        std::fs::write(self.root.path().join("brief-release"), "").unwrap();
    }
    fn envelope(&self) -> String {
        std::fs::read_to_string(self.root.path().join("final-provider-envelope")).unwrap()
    }
    fn provider_inputs(&self) -> String {
        // Claude delivers hidden context through its hook file, separately from
        // the visible stdin message. Inspect both actual provider input surfaces.
        format!(
            "{}\n{}",
            self.envelope(),
            std::fs::read_to_string(self.root.path().join("provider-hidden-context")).unwrap(),
        )
    }
}
impl Drop for PausedBrief {
    fn drop(&mut self) {
        let _ = self
            .runtime
            .owned
            .provider_store
            .terminate_run_provider_only(&self.session_id, self.run.id());
    }
}

#[tokio::test]
async fn final_handoff_envelope_rechecks_room_registration_during_brief() {
    let fixture = PausedBrief::new(false, false).await;
    fixture.prepare().await;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    assert!(
        !fixture.envelope().contains(SECRET),
        "final provider envelope replayed history captured before Room registration"
    );
}

#[tokio::test]
async fn final_handoff_envelope_protects_failed_request_note_during_brief() {
    let fixture = PausedBrief::new(false, false).await;
    fixture
        .runtime
        .owned
        .agent_store
        .record_failed_request_durably(
            &fixture.runtime.owned.durable_state_store,
            &fixture.agent_id,
            crate::agent::FailedRequest::new(
                "failed-before-switch",
                SECRET,
                "provider error".into(),
            ),
        )
        .unwrap();
    fixture.prepare().await;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    let envelope = fixture.provider_inputs();
    assert!(envelope.contains("Your previous request"));
    assert!(
        !envelope.contains(SECRET),
        "retained failed-request note leaked in the actual provider envelope"
    );
}

#[tokio::test]
async fn final_handoff_envelope_protects_scheduled_hidden_context_during_brief() {
    let fixture = PausedBrief::new(false, false).await;
    fixture
        .prepare_with_inputs(format!("scheduled-context-marker {SECRET}"), Vec::new())
        .await;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    let envelope = fixture.provider_inputs();
    assert!(envelope.contains("scheduled-context-marker"));
    assert!(
        !envelope.contains(SECRET),
        "retained scheduled context leaked in the actual provider envelope"
    );
}

#[tokio::test]
async fn final_handoff_envelope_protects_attachments_during_brief() {
    let fixture = PausedBrief::new(false, false).await;
    let attachment = crate::session::PromptAttachment::new(
        format!("https://example.com/{SECRET}"),
        "text/plain",
        Some(format!("attachment-marker-{SECRET}.txt")),
    );
    fixture
        .prepare_with_inputs(String::new(), vec![attachment])
        .await;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    let envelope = fixture.provider_inputs();
    assert!(envelope.contains("attachment-marker"));
    assert!(
        !envelope.contains(SECRET),
        "retained attachment leaked in the actual provider envelope"
    );
}

#[tokio::test]
async fn final_handoff_envelope_scrubs_already_registered_queued_session_facts() {
    let fixture = PausedBrief::new(true, false).await;
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let queued = PromptQueueItem::new(
        "protected-queued",
        &fixture.attachment_id,
        &fixture.agent_id,
        SECRET,
        PromptStatus::Queued,
    );
    assert!(matches!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(&session, queued, false)
            .unwrap(),
        PromptSubmissionOutcome::Queued { .. }
    ));
    fixture.prepare().await;
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    assert!(
        !fixture.envelope().contains(SECRET),
        "final provider envelope leaked a protected queued fact"
    );
}

#[tokio::test]
async fn cancellation_during_brief_promotes_followup_through_shared_preparation() {
    let fixture = PausedBrief::new(false, true).await;
    fixture.prepare().await;
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let queued = PromptQueueItem::new(
        "queued-followup",
        &fixture.attachment_id,
        &fixture.agent_id,
        "queued followup",
        PromptStatus::Queued,
    );
    assert!(matches!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(&session, queued, false)
            .unwrap(),
        PromptSubmissionOutcome::Queued { .. }
    ));
    fixture
        .runtime
        .owned
        .prompt_state_owner
        .begin_cancelling_prompt_if_matches(&session, &fixture.agent_id, Some(&fixture.prompt_id))
        .unwrap();
    let cancellation = fixture
        .runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            &fixture.session_id,
            &fixture.agent_id,
            &fixture.prompt_id,
            Some(fixture.run.id()),
        )
        .unwrap();
    let dispatch = cancellation
        .dispatch
        .expect("cancellation promotion bypassed shared brief preparation");
    fixture.release();
    fixture
        .runtime
        .enqueue_prompt_dispatch(&dispatch)
        .await
        .unwrap();
    fixture.wait_for("final-provider-envelope").await;
    let envelope = fixture.envelope();
    assert!(
        !envelope.contains("brief-only-amber") && envelope.contains("notes notes"),
        "fitting followup must preserve verbatim history without re-inserting a stale brief"
    );
    assert!(
        !envelope.contains("cancelled request"),
        "cancelled request was submitted after its fold settled"
    );
}

#[tokio::test]
async fn final_handoff_rebuilds_history_after_room_retirement_during_brief() {
    let fixture = PausedBrief::new(true, false).await;
    fixture.prepare().await;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .forget(&fixture.session_id)
        .unwrap();
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    let envelope = fixture.envelope();
    assert!(
        !envelope.contains("notes notes"),
        "retired Room history was replayed from the old snapshot"
    );
    assert!(
        !envelope.contains("brief-only-amber"),
        "recovered Room brief cache was replayed"
    );
}

#[tokio::test]
async fn final_handoff_rejects_room_fenced_during_brief() {
    let fixture = PausedBrief::new(false, false).await;
    fixture.prepare().await;
    assert!(fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, "")
        .is_err());
    fixture.release();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = fixture
                .runtime
                .owned
                .session_store
                .get_session(&fixture.session_id)
                .unwrap();
            if fixture
                .runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &fixture.agent_id)
                .is_none()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fenced handoff must settle without provider submission");
    assert!(!fixture.root.path().join("final-provider-envelope").exists());
}

#[tokio::test]
async fn explicit_handoff_rebuilds_after_room_policy_changed_after_capture() {
    let fixture = PausedBrief::new(false, false).await;
    let mut handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &fixture.run,
            &fixture.prompt_id,
            false,
        )
        .unwrap()
        .unwrap();
    handoff.derived = false;
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    let dispatch = dispatch(
        &fixture.session_id,
        &fixture.agent_id,
        &fixture.attachment_id,
        fixture.run.id(),
        &fixture.prompt_id,
        "next request",
        None,
        false,
    );
    let result = fixture
        .runtime
        .submit_structured_prompt_with_handoff(&dispatch, &fixture.run, Some(handoff), "")
        .await;
    result.unwrap();
    fixture.wait_for("final-provider-envelope").await;
    assert!(
        !fixture.envelope().contains(SECRET),
        "explicit handoff failed to rebuild protected history"
    );
}

#[tokio::test]
async fn final_handoff_waits_for_room_input_before_rebuilding_protected_history() {
    let fixture = PausedBrief::new(false, false).await;
    fixture.prepare().await;
    let barrier = fixture
        .runtime
        .owned
        .room_secret_observations
        .barrier(&fixture.session_id)
        .unwrap();
    let guard = barrier.write_owned().await;
    fixture.release();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !fixture.root.path().join("final-provider-envelope").exists(),
        "handoff bypassed the Room input barrier"
    );
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    drop(guard);
    fixture.wait_for("final-provider-envelope").await;
    assert!(!fixture.envelope().contains(SECRET));
}

// MP-08 / MP-10 / MP-11: the new native policy must inherit explicit-snapshot replay fences.
#[tokio::test]
async fn native_policy_rebuilds_explicit_snapshot_after_room_change() {
    let fixture = PausedBrief::new(false, false).await;
    let mut handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &fixture.run,
            &fixture.prompt_id,
            false,
        )
        .unwrap()
        .unwrap();
    handoff.derived = false;
    fixture.runtime.owned.pending_agent_context_handoffs.set(
        &fixture.session_id,
        &fixture.agent_id,
        handoff,
    );
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    let result = crate::app::NativePromptHandoffPolicy::prepare_context(
        &fixture.runtime.owned,
        &fixture.session_id,
        &fixture.agent_id,
        fixture.run.id(),
        &fixture.prompt_id,
        false,
        String::new(),
    );
    let context = result.unwrap();
    assert!(
        !context.contains(SECRET),
        "native replay ignored changed Room protection"
    );
}

// MP-08 / MP-10 / MP-11: hook-time rebuild must exclude the managed request already sent visibly.
#[tokio::test]
async fn native_hook_rebuild_excludes_the_current_managed_prompt() {
    let fixture = PausedBrief::new(false, false).await;
    fixture
        .runtime
        .owned
        .operational_history_store
        .append(&crate::history::HistoryEvent::transcript(
            200,
            &crate::history::SessionHistoryEntry::user_prompt(
                &fixture.session_id,
                "current",
                &fixture.agent_id,
                "unique-current-managed-request",
            ),
            crate::history::HistoryEventTurnContext {
                prompt_id: Some(fixture.prompt_id.clone()),
                ..Default::default()
            },
        ))
        .unwrap();
    let context = crate::app::NativePromptHandoffPolicy::prepare_context(
        &fixture.runtime.owned,
        &fixture.session_id,
        &fixture.agent_id,
        fixture.run.id(),
        &fixture.prompt_id,
        false,
        String::new(),
    )
    .unwrap();
    assert!(
        !context.contains("unique-current-managed-request"),
        "fresh native hook replayed the current managed request in its handoff"
    );
}

#[tokio::test]
async fn room_retirement_between_brief_chunks_never_replays_the_next_utility_input() {
    let (root, runtime, session_id, agent_id, attachment_id, prompt_id, _) =
        runtime_with_active_prompt_and_history(Some("retire-this-transcript")).await;
    for index in 0..100 {
        let prior = format!("prior-review-{index}");
        runtime
            .owned
            .operational_history_store
            .append(&crate::history::HistoryEvent::transcript(
                100,
                &crate::history::SessionHistoryEntry::user_prompt(
                    &session_id,
                    &prior,
                    &agent_id,
                    format!(
                        "retire-this-transcript {index} {}",
                        "historical review record ".repeat(100)
                    ),
                ),
                crate::history::HistoryEventTurnContext {
                    prompt_id: Some(prior),
                    ..Default::default()
                },
            ))
            .unwrap();
    }
    let fixture = PausedBrief::from_history(
        root,
        runtime,
        session_id,
        agent_id,
        attachment_id,
        prompt_id,
        // Haiku has a stable 200k window; default Sonnet grows to 1M in #919.
        "haiku",
    );
    fixture.prepare().await;
    {
        let _guard = fixture
            .runtime
            .owned
            .room_secret_observations
            .barrier(&fixture.session_id)
            .unwrap()
            .write_owned()
            .await;
        fixture
            .runtime
            .owned
            .room_secret_observations
            .forget(&fixture.session_id)
            .unwrap();
    }
    fixture.release();
    fixture.wait_for("final-provider-envelope").await;
    let inputs = std::fs::read_to_string(fixture.root.path().join("brief-utility-inputs")).unwrap();
    assert!(
        inputs.lines().next().unwrap().contains("1 of 2"),
        "the regression must exercise two real rendered utility chunks"
    );
    assert_eq!(
        inputs.lines().count(),
        1,
        "a second utility replayed a captured transcript after Room retirement"
    );
    assert!(!fixture.envelope().contains("retire-this-transcript"));
    assert!(!fixture.envelope().contains("brief-only-amber"));
    assert!(
        fixture
            .runtime
            .owned
            .operational_history_store
            .load_agent_handoff_brief(&fixture.session_id, &fixture.agent_id)
            .unwrap()
            .is_none(),
        "retired fold kept its watermark"
    );
}

#[tokio::test]
async fn native_utility_protection_gate_runs_before_any_prompt_write() {
    let fixture = PausedBrief::new(false, false).await;
    let scratch = super::super::super::project_environment_export::MetadataUtilityScratch::new(
        &fixture
            .runtime
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root(),
    )
    .unwrap();
    let mut run = fixture.run.clone();
    run.set_metadata_only_discovery(scratch.0.clone());
    let admission = crate::provider::ProviderUtilityAdmission::new(|| {
        Err(DaemonError::LocalTransport {
            operation: "utility source protection test",
            message: "source policy changed".into(),
        })
    });
    let error = crate::runtime::agent_utility_executor::run_provider_utility_prompt_with_admission(
        &fixture.runtime,
        run,
        crate::runtime::agent_utility_executor::AgentUtilityPromptParts {
            visible_user_prompt: "Update the handoff brief with protected source".into(),
            hidden_system_context: String::new(),
        },
        "utility source protection test",
        crate::provider::ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
        std::time::Duration::from_secs(2),
        Some(admission),
    )
    .await
    .expect_err("changed source must be rejected at the native writer");
    assert!(
        matches!(
            error,
            DaemonError::LocalTransport {
                operation: "utility source protection test",
                ..
            }
        ),
        "the actor did not reach the source-admission seam: {error}"
    );
    assert!(!fixture.root.path().join("brief-started").exists());
    assert!(!fixture.root.path().join("brief-utility-inputs").exists());
}
