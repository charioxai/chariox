//! MP-08 / MP-10 / MP-11 A04: hour-scale windows bound to owner work.
use super::tests::{fixture_with_options, popup, running, Fixture, PASSKEY};
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome};

pub(super) async fn approve(
    f: &Fixture,
    prompt: &PasskeyPrompt,
    minutes: Option<&str>,
) -> Result<(), DaemonError> {
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            minutes,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
}

/// Opens a real window through the terminal popup and returns it.
pub(super) async fn open(f: &Fixture, minutes: Option<&str>) -> KernelSudoTurn {
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    approve(f, &prompt, minutes).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    f.state
        .list_sudo_turns("local")
        .pop()
        .expect("window is live")
}

fn session(f: &Fixture) -> crate::session::RuntimeSession {
    f.state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
}

fn outcomes(f: &Fixture, entry: &str) -> Vec<String> {
    f.state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(entry, "kernel_access.sudo", 100)
        .unwrap()
        .into_iter()
        .map(|event| {
            event.payload["outcome"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Starts a kernel-correlated continuation of the window's task after its
/// elevated turn ended; returns the continuation prompt id.
pub(super) fn start_continuation(f: &Fixture, window: &KernelSudoTurn) -> String {
    let agent = window.agent_id.clone();
    let task = window.task_id.clone().unwrap();
    let Outcome::Task(blocked) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: task.clone(),
            prompt: task.clone(),
            reason: "fixture".into(),
        })
        .unwrap()
    else {
        unreachable!()
    };
    let Outcome::Task(resumed) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::OwnerResponse {
            task: task.clone(),
            revision: blocked.blocked_revision,
            resume: true,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        unreachable!()
    };
    let continuation = resumed.pending_prompt_id.clone().unwrap();
    let prepared = crate::app::KernelPreparedPromptSubmission {
        session_id: window.session_id.clone(),
        prompt: PromptQueueItem::new(
            &continuation,
            &f.request.attachment_id,
            &agent,
            "continue",
            PromptStatus::Queued,
        )
        .with_durable_operation(&continuation, format!("task:{task}:{}", resumed.revision)),
        force_queue: false,
        refresh_projection: true,
    };
    f.state.owned.admit_agent_task(&prepared).unwrap();
    let outcome = f
        .state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&prepared, true)
        .unwrap()
        .unwrap()
        .outcome;
    assert!(
        matches!(outcome, PromptSubmissionOutcome::Started { .. }),
        "{outcome:?}"
    );
    continuation
}

#[tokio::test]
async fn sudo_window_projection_keeps_deadline_and_warning_after_mutation() {
    let f = fixture_with_options(None, true);
    let window = running(&f);
    let plain = session(&f);
    assert!(plain.sudo_windows().is_empty());
    let projected = f.state.owned.update_session_projection(plain.clone());
    assert_eq!(
        projected.sudo_windows().len(),
        1,
        "ordinary projection refreshes must retain the live sudo window"
    );
    let current = f
        .state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .map(|current| {
            current.warning_sent = true;
            current.clone()
        })
        .unwrap();
    let projected = f.state.owned.update_session_projection(plain);
    assert_eq!(projected.sudo_windows(), &[current]);
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
    let projected = f.state.owned.update_session_projection(session(&f));
    assert!(projected.sudo_windows().is_empty());
}

#[tokio::test]
async fn sudo_windows_are_projected_only_to_their_owner() {
    let f = fixture_with_options(None, true);
    let window = running(&f);
    let projected = f.state.owned.update_session_projection(session(&f));
    assert_eq!(
        projected.clone().redacted_for_user("local").sudo_windows(),
        &[window.clone()]
    );
    assert!(
        projected
            .redacted_for_user("member")
            .sudo_windows()
            .is_empty(),
        "other members must not receive window, run or requester details"
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

// MP-08/MP-10/MP-11: App read/publish paths must project the same kernel window.
#[tokio::test]
async fn app_session_snapshots_keep_the_current_kernel_sudo_window() {
    let f = fixture_with_options(None, true);
    let window = running(&f);
    let _ = f.state.owned.session_snapshot(&window.session_id).unwrap();
    let app = f.app.lock().await;
    let snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&window.session_id)
        .unwrap();
    assert_eq!(
        snapshot.sudo_windows(),
        &[window.clone()],
        "App snapshots must carry the kernel window on late attach"
    );
    app.update_session_projection(session(&f));
    let projected = app
        .session_state_projection_store()
        .get(&window.session_id)
        .unwrap();
    assert_eq!(
        projected.sudo_windows(),
        &[window],
        "App refreshes must not erase a current kernel window"
    );
}

#[tokio::test]
async fn sudo_popup_defaults_to_one_hour_and_refuses_more_than_eight() {
    let f = fixture_with_options(None, true);
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    assert_eq!(prompt.lifetime_minutes, Some(60));
    assert_eq!(prompt.max_lifetime_minutes, Some(480));
    for refused in ["600", "90", "0"] {
        assert!(
            approve(&f, &prompt, Some(refused)).await.is_err(),
            "{refused} minutes"
        );
    }
    approve(&f, &prompt, Some("120")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let window = f.state.list_sudo_turns("local").pop().unwrap();
    assert_eq!((window.duration_minutes, window.revision), (120, 1));
    let left = window.expires_at_ms.unwrap() - crate::session::unix_epoch_ms();
    assert!((7_190_000..=7_200_000).contains(&left), "{left}");
    assert_eq!(window.task_id.as_deref(), window.prompt_id.as_deref());
    // Every client's status row reads the kernel deadline from the snapshot.
    let snapshot = f
        .state
        .owned
        .session_snapshot(&f.request.session_id)
        .unwrap();
    assert_eq!(snapshot.sudo_windows(), [window.clone()]);
    assert!(sudo_window_minutes(None).is_ok_and(|minutes| minutes == 60));
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_window_survives_waits_and_admits_only_its_own_work() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let agent = window.agent_id.clone();
    let old_command = f.state.with_external_command_authority(Some((
        &window.entry_id,
        &LocalDaemonRequest::ListSessions(ListSessionsRequest),
    )));
    assert!(old_command.authorize_current_external_command().is_ok());
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_ok());
    // The elevated turn ends without finishing its work: the window stays,
    // but no turn of that work is running, so nothing is authorized.
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session(&f), &agent)
        .unwrap();
    f.state.sweep_sudo();
    assert_eq!(f.state.list_sudo_turns("local").len(), 1);
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    assert!(f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    // An unrelated owner prompt stays queued: it never enters elevated context.
    let unrelated = crate::app::KernelPreparedPromptSubmission {
        session_id: window.session_id.clone(),
        prompt: PromptQueueItem::new(
            "unrelated-owner-prompt",
            &f.request.attachment_id,
            &agent,
            "unrelated",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    let outcome = f
        .state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&unrelated, true)
        .unwrap()
        .unwrap()
        .outcome;
    assert!(
        matches!(outcome, PromptSubmissionOutcome::Queued { .. }),
        "{outcome:?}"
    );
    assert!(f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session(&f), &agent)
        .is_none());
    // A kernel-correlated continuation of the same task starts and is bound.
    let continuation = start_continuation(&f, &window);
    let resumed_turn = f.state.sudo_for_auth_token("sudo-fixture-bearer").unwrap();
    assert_eq!(resumed_turn.entry_id, window.entry_id);
    assert_eq!(
        resumed_turn.prompt_id.as_deref(),
        Some(continuation.as_str())
    );
    assert!(
        old_command.authorize_current_external_command().is_err(),
        "an in-flight command from the old turn cannot acquire the continuation's authority"
    );
    assert!(
        old_command
            .with_external_command_authority(Some((
                &window.entry_id,
                &LocalDaemonRequest::ListSessions(ListSessionsRequest)
            )))
            .authorize_current_external_command()
            .is_err(),
        "re-scoping the same grant cannot renew an old command's turn"
    );
    assert!(f
        .state
        .with_external_command_authority(Some((
            &window.entry_id,
            &LocalDaemonRequest::ListSessions(ListSessionsRequest)
        )))
        .authorize_current_external_command()
        .is_ok());
    // Expiry ends authority but not the running regular work, and releases
    // the deferred prompt for a distinct regular turn.
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now());
    f.state.pump_sudo_windows(None);
    assert!(f.state.list_sudo_turns("local").is_empty());
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    let active = f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session(&f), &agent)
        .unwrap();
    assert_eq!(active.id(), continuation);
    assert!(!f
        .state
        .owned
        .prompt_state_owner
        .sudo_work_held(&session(&f), &agent));
    assert!(outcomes(&f, &window.entry_id).contains(&"expired".to_owned()));
    // MP-08/MP-10/MP-11: main advertises the dormant interface before the
    // first provider turn; expiry must revoke authority, not discovery.
    assert!(f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    assert!(f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"ListSessions":null}})
        )
        .await
        .is_err());
}

#[tokio::test]
async fn sudo_window_ends_when_its_owner_work_ends() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let task_id = window.task_id.clone().unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session(&f), &window.agent_id)
        .unwrap();
    let task = f
        .state
        .owned
        .durable_state_store
        .agent_tasks(Some(&window.session_id), Some(&window.agent_id))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == task_id)
        .unwrap();
    f.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CancelTask {
            task: task_id,
            owner: "local".into(),
            revision: task.revision,
        })
        .unwrap();
    f.state.sweep_sudo();
    assert!(f.state.list_sudo_turns("local").is_empty());
    assert!(outcomes(&f, &window.entry_id).contains(&"work_ended".to_owned()));
    assert!(!f
        .state
        .owned
        .prompt_state_owner
        .sudo_work_held(&session(&f), &window.agent_id));
}

fn deferred_prompt_notices(f: &Fixture) -> usize {
    f.state
        .owned
        .operational_history_store
        .load_session_history_entries(&f.request.session_id, None)
        .unwrap()
        .iter()
        .filter(|entry| format!("{entry:?}").contains("Deferred prompt"))
        .count()
}

#[tokio::test]
async fn sudo_start_does_not_report_its_own_prompt_as_deferred() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    assert_eq!(deferred_prompt_notices(&f), 0);
    let unrelated = crate::app::KernelPreparedPromptSubmission {
        session_id: window.session_id.clone(),
        prompt: PromptQueueItem::new(
            "unrelated-owner-prompt",
            &f.request.attachment_id,
            &window.agent_id,
            "unrelated",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    f.state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&unrelated, true)
        .unwrap();
    assert_eq!(deferred_prompt_notices(&f), 1);
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_extension_before_the_first_turn_keeps_the_window() {
    let f = fixture_with_options(None, true);
    let agent = f.request.target_agent_id.clone().unwrap();
    // The agent is busy with ordinary work, so the authorized window waits.
    let busy = crate::app::KernelPreparedPromptSubmission {
        session_id: f.request.session_id.clone(),
        prompt: PromptQueueItem::new(
            "ordinary-busy-prompt",
            &f.request.attachment_id,
            &agent,
            "ordinary",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    f.state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&busy, true)
        .unwrap();
    let state = f.state.clone();
    let request = f.request.clone();
    let start = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    approve(&f, &popup(&f.state).await, None).await.unwrap();
    let queued = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(turn) = f.state.list_sudo_turns("local").pop() {
                if turn.deadline.is_some() {
                    return turn;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(queued.prompt_id.is_none());
    // A provider that is already running must see the tool before the first
    // elevated turn starts; calls stay refused until that turn is bound.
    assert!(f
        .router
        .runtime_tool_specs_for_auth_token("sudo-fixture-bearer")
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request"));
    assert!(f.state.sudo_for_auth_token("sudo-fixture-bearer").is_err());
    let state = f.state.clone();
    let extend = tokio::spawn({
        let request = ExtendKernelSudoRequest {
            session_id: queued.session_id.clone(),
            attachment_id: f.request.attachment_id.clone(),
            entry_id: queued.entry_id.clone(),
            revision: queued.revision,
        };
        async move { state.extend_sudo_window(request, "local").await }
    });
    let prompt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = f
                .state
                .passkey_prompts_for("local")
                .into_iter()
                .find(|p| p.interaction_id.contains(":extend:"))
            {
                return prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    approve(&f, &prompt, Some("120")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), extend)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // The busy turn ends; the extended window's first turn starts.
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session(&f), &agent)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), start)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let live = f.state.list_sudo_turns("local").pop().expect("window kept");
    assert_eq!(
        (live.revision, live.duration_minutes),
        (queued.revision + 1, 120)
    );
    assert_eq!(live.prompt_id.as_deref(), Some(queued.entry_id.as_str()));
    assert!(!outcomes(&f, &queued.entry_id).contains(&"refused_or_cancelled".to_owned()));
    f.state
        .revoke_sudo(Some("local"), Some(&queued.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_extend_needs_a_fresh_passkey_and_settles_once() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let stale = ExtendKernelSudoRequest {
        session_id: window.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        entry_id: window.entry_id.clone(),
        revision: 0,
    };
    assert!(f.state.extend_sudo_window(stale, "local").await.is_err());
    let guest = ExtendKernelSudoRequest {
        session_id: window.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        entry_id: window.entry_id.clone(),
        revision: 1,
    };
    assert!(f
        .state
        .extend_sudo_window(guest.clone(), "guest")
        .await
        .is_err());
    let state = f.state.clone();
    let extend = tokio::spawn(async move { state.extend_sudo_window(guest, "local").await });
    let prompt = popup(&f.state).await;
    assert_eq!(
        prompt.interaction_id,
        format!("{}:extend:1", window.entry_id)
    );
    // A wrong passkey or a remembered presence leaves the deadline unchanged.
    let wrong = f
        .state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            Some("240"),
            Some("local"),
            Some(&ApprovalPasskey::new("wrong")),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await;
    assert!(wrong.is_err());
    let remembered = f
        .state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            Some("240"),
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            Some(5),
            Some(KernelConnectionClass::Terminal),
        )
        .await;
    assert!(remembered.is_err());
    assert_eq!(f.state.list_sudo_turns("local")[0].revision, 1);
    approve(&f, &prompt, Some("240")).await.unwrap();
    let LocalDaemonResponse::KernelSudoExtended { turn } =
        tokio::time::timeout(Duration::from_secs(5), extend)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    else {
        panic!("extension")
    };
    assert_eq!(
        (turn.revision, turn.duration_minutes, turn.warning_sent),
        (2, 240, false)
    );
    let left = turn.expires_at_ms.unwrap() - crate::session::unix_epoch_ms();
    assert!((14_390_000..=14_400_000).contains(&left), "{left}");
    // The old revision cannot be replayed into another extension.
    assert!(approve(&f, &prompt, Some("480")).await.is_err());
    let replay = ExtendKernelSudoRequest {
        session_id: window.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        entry_id: window.entry_id.clone(),
        revision: 1,
    };
    assert!(f.state.extend_sudo_window(replay, "local").await.is_err());
    assert!(outcomes(&f, &window.entry_id).contains(&"extended".to_owned()));
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
    let ended = ExtendKernelSudoRequest {
        session_id: window.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        entry_id: window.entry_id.clone(),
        revision: 2,
    };
    assert!(
        f.state.extend_sudo_window(ended, "local").await.is_err(),
        "revoked windows never resurrect"
    );
}

#[tokio::test]
async fn sudo_warns_once_per_revision_and_proves_its_timer_is_live() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    // Proof of life: the armed timer task records its own revision.
    tokio::time::timeout(Duration::from_secs(5), async {
        while f
            .state
            .owned
            .sudo_timers
            .lock()
            .unwrap()
            .get(&window.entry_id)
            != Some(&(1, 0))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now() + Duration::from_secs(9 * 60));
    f.state.pump_sudo_windows(None);
    f.state.pump_sudo_windows(None);
    let warnings = outcomes(&f, &window.entry_id)
        .iter()
        .filter(|o| *o == "warning")
        .count();
    assert_eq!(warnings, 1);
    assert!(outcomes(&f, &window.entry_id).contains(&"timer_armed".to_owned()));
    assert!(
        f.state
            .owned
            .session_snapshot(&window.session_id)
            .unwrap()
            .sudo_windows()[0]
            .warning_sent
    );
    // A timer that never armed is reported by the sweep, never assumed.
    f.state
        .owned
        .sudo_timers
        .lock()
        .unwrap()
        .remove(&window.entry_id);
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now() + Duration::from_secs(60 * 60 - 6));
    f.state.pump_sudo_windows(None);
    f.state.pump_sudo_windows(None);
    let missed = outcomes(&f, &window.entry_id)
        .iter()
        .filter(|o| *o == "timer_arm_missed")
        .count();
    assert_eq!(missed, 1);
    // An expiry the timer missed is enforced by the sweep with an alert.
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now() - Duration::from_secs(10));
    f.state.pump_sudo_windows(None);
    let outcomes = outcomes(&f, &window.entry_id);
    assert!(
        outcomes.contains(&"timer_expiry_missed".to_owned()),
        "{outcomes:?}"
    );
    assert!(outcomes.contains(&"expired".to_owned()));
}

#[tokio::test]
async fn sudo_agents_cannot_answer_any_approval() {
    let f = fixture_with_options(None, true);
    let turn = running(&f);
    let responder = f
        .state
        .create_kernel_operation_interaction(
            &turn.session_id,
            "local",
            RuntimeInteraction::for_kernel_operation(
                "critical-payment",
                "payment",
                "Payment",
                "Fixture only",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .unwrap();
    let answer = serde_json::json!({"RespondToInteraction": {"session_id": turn.session_id, "interaction_id": "critical-payment", "choice_id": "deny", "custom_reply": null, "passkey": null, "passkey_remember_minutes": null}});
    let request: LocalDaemonRequest = serde_json::from_value(answer.clone()).unwrap();
    assert!(f
        .state
        .authorize_sudo_request(&turn.entry_id, &request)
        .is_err());
    assert!(f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request": answer})
        )
        .await
        .is_err());
    // The legacy Meta resolver is gone under every alias.
    for name in [
        "chariox.meta.resolve_runtime_interaction",
        "chariox_meta_resolve_runtime_interaction",
        "mcp__chariox__meta_resolve_runtime_interaction",
    ] {
        assert!(
            crate::transport::runtime_tools::canonical_meta_tool_name(name).is_none(),
            "{name}"
        );
    }
    assert!(!crate::transport::runtime_tools::meta_runtime_tool_specs()
        .iter()
        .any(|spec| spec.name.contains("resolve_runtime_interaction")));
    // The owner still answers once through the client.
    f.state
        .answer_terminal_runtime_interaction(
            &turn.session_id,
            "critical-payment",
            "deny",
            None,
            Some("local"),
            None,
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    assert_eq!(responder.await.unwrap().choice_id.as_deref(), Some("deny"));
}

#[test]
fn sudo_fence_lets_correlated_wakes_pass_deferred_messages() {
    let worktree = crate::test_support::TestWorktree::new("sudo-fence-ledger");
    let store = crate::durable_state::DurableKernelStateStore::open_owned(
        worktree.path().join("state.sqlite3"),
    )
    .unwrap();
    let occur = |id: &str, kind: &str, payload: serde_json::Value| {
        let Outcome::Event(event) = store
            .agent_lifecycle(Operation::Occur(ledger::occurrence(
                "room", "agent", "source", id, kind, payload,
            )))
            .unwrap()
        else {
            unreachable!()
        };
        event
    };
    let message = occur(
        "m1",
        "message",
        serde_json::json!({"message": "unrelated", "task_id": "work"}),
    );
    let wake = occur("w1", "sudo_ended", serde_json::json!({"task_id": "work"}));
    assert_eq!(
        store
            .agent_delivery_front("room", "agent")
            .unwrap()
            .unwrap()
            .sequence,
        message.sequence
    );
    assert_eq!(
        store
            .agent_work_delivery_front("room", "agent", Some("work"))
            .unwrap()
            .unwrap()
            .sequence,
        wake.sequence
    );
    let attempt = |work: Option<&str>| {
        store.agent_lifecycle(Operation::Attempt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: wake.sequence,
            prompt: "wake".into(),
            target: None,
            run: None,
            now: 1,
            work: work.map(str::to_owned),
        })
    };
    assert!(
        attempt(None).is_err(),
        "without the fence FIFO order still applies"
    );
    assert!(attempt(Some("work")).is_ok());
    let forged = store.agent_lifecycle(Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: message.sequence,
        prompt: "peer-request".into(),
        target: None,
        run: None,
        now: 1,
        work: Some("work".into()),
    });
    assert!(
        forged.is_err(),
        "an unrelated event cannot claim the work binding"
    );
}

#[tokio::test]
async fn sudo_deferred_events_outlast_the_delivery_timeout_and_run_after_the_window() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let (room, agent) = (window.session_id.clone(), window.agent_id.clone());
    let work = window.task_id.clone().unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session(&f), &agent)
        .unwrap();
    let store = &f.state.owned.durable_state_store;
    let occur = |id: &str, kind: &str, payload: serde_json::Value| {
        let Outcome::Event(event) = store
            .agent_lifecycle(Operation::Occur(ledger::occurrence(
                &room, &agent, "peer", id, kind, payload,
            )))
            .unwrap()
        else {
            unreachable!()
        };
        event
    };
    // An unrelated event whose idle refusal clock started before the window
    // and is now older than the delivery timeout.
    let unrelated = occur("peer-1", "message", serde_json::json!({"message": "later"}));
    let stale = crate::session::unix_epoch_ms() - ledger::DELIVERY_TIMEOUT_MS - 1_000;
    for op in [
        Operation::Attempt {
            room: room.clone(),
            agent: agent.clone(),
            sequence: unrelated.sequence,
            prompt: "refused".into(),
            target: None,
            run: None,
            now: stale,
            work: None,
        },
        Operation::Receipt {
            room: room.clone(),
            agent: agent.clone(),
            sequence: unrelated.sequence,
            state: "rejected".into(),
            now: stale,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    f.state.sweep_agent_lifecycle().await.unwrap();
    let held = store.agent_inbox(&room, &agent, 0).unwrap();
    assert_eq!(
        held[0].state, "pending",
        "deferral is waiting, not a lost delivery"
    );
    assert!(store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .iter()
        .all(|t| t.task_id != format!("delivery-{}", unrelated.sequence)));
    // The elevated task's own wakes still pass the deferred event.
    let wake = occur("wake-1", "sudo_ended", serde_json::json!({"task_id": work}));
    assert_eq!(
        store
            .agent_work_delivery_front(&room, &agent, Some(&work))
            .unwrap()
            .unwrap()
            .sequence,
        wake.sequence
    );
    // Once the window ends the deferred event runs as a regular turn.
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
    f.state.deliver_agent_inbox(&room, &agent).await.unwrap();
    let delivered = store.agent_inbox(&room, &agent, 0).unwrap();
    assert!(
        matches!(delivered[0].state.as_str(), "submitting" | "accepted"),
        "{:?}",
        delivered[0].state
    );
}

#[tokio::test]
async fn sudo_inbox_read_cannot_import_unrelated_peer_context() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let task = window.task_id.as_deref().unwrap();
    let message = ledger::occurrence(
        &window.session_id,
        &window.agent_id,
        "peer",
        "peer-message",
        "message",
        serde_json::json!({"task_id": task, "message": "unrelated peer work"}),
    );
    f.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(message))
        .unwrap();
    let result = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.inbox",
            serde_json::json!({"origin_prompt_id": window.prompt_id, "task_id": task, "after": 0}),
        )
        .await
        .unwrap();
    assert!(
        result.payload.as_array().unwrap().is_empty(),
        "unrelated events must stay outside the elevated context"
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_warning_is_retried_when_its_durable_receipt_fails() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let database = rusqlite::Connection::open(f.state.owned.durable_state_store.path()).unwrap();
    database.execute_batch("CREATE TRIGGER reject_sudo_warning BEFORE INSERT ON durable_state_events WHEN NEW.kind = 'kernel_access.sudo' AND json_extract(NEW.payload_json, '$.outcome') = 'warning' BEGIN SELECT RAISE(ABORT, 'injected warning failure'); END;").unwrap();
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now() + Duration::from_secs(9 * 60));
    f.state.pump_sudo_windows(None);
    assert!(
        !f.state.list_sudo_turns("local")[0].warning_sent,
        "failed persistence must leave the warning due"
    );
    database
        .execute_batch("DROP TRIGGER reject_sudo_warning;")
        .unwrap();
    f.state.pump_sudo_windows(None);
    assert!(f.state.list_sudo_turns("local")[0].warning_sent);
    assert_eq!(
        outcomes(&f, &window.entry_id)
            .iter()
            .filter(|o| *o == "warning")
            .count(),
        1
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_scope_cannot_be_widened_by_results_or_extension() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let request = LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
        path: "workflow.session_default_max_agents".into(),
        value: "16".into(),
    });
    assert!(f
        .state
        .authorize_sudo_request(&window.entry_id, &request)
        .is_err());
    let state = f.state.clone();
    let initial = window.clone();
    let operation = request.clone();
    let confirmation =
        tokio::spawn(async move { state.confirm_sudo_scope(&initial, &operation).await });
    let prompt = popup(&f.state).await;
    assert!(prompt.interaction_id.contains(":scope:"));
    assert_eq!(prompt.lifetime_minutes, None);
    assert!(f
        .state
        .answer_terminal_runtime_interaction(
            &window.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            None,
            Some(5),
            Some(KernelConnectionClass::Terminal)
        )
        .await
        .is_err());
    approve(&f, &prompt, None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), confirmation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(f
        .state
        .authorize_sudo_request(&window.entry_id, &request)
        .is_ok());
    let changed = LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
        path: "workflow.session_default_max_agents".into(),
        value: "17".into(),
    });
    assert!(f
        .state
        .authorize_sudo_request(&window.entry_id, &changed)
        .is_err());
    // A time-only extension retains exactly the original approved scope.
    super::window::open_window(
        f.state
            .owned
            .sudo_turns
            .lock()
            .unwrap()
            .get_mut(&window.entry_id)
            .unwrap(),
        120,
    );
    assert!(f
        .state
        .authorize_sudo_request(&window.entry_id, &request)
        .is_ok());
    assert!(f
        .state
        .authorize_sudo_request(&window.entry_id, &changed)
        .is_err());
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
    assert!(f.state.owned.sudo_scopes.lock().unwrap().is_empty());
}

// MP-08/MP-10/MP-11: scope approval is not authority to finish an effect
// after revocation while waiting for the app mutex.
#[tokio::test]
async fn sudo_config_effect_rechecks_authority_after_app_lock_wait() {
    config_effect_after_app_lock_wait(false).await;
}

#[tokio::test]
async fn sudo_config_effect_rechecks_expiry_after_app_lock_wait() {
    config_effect_after_app_lock_wait(true).await;
}

async fn config_effect_after_app_lock_wait(expired: bool) {
    let f = fixture_with_options(None, true);
    let window = running(&f);
    let request = LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
        path: "workflow.session_default_max_agents".into(),
        value: "16".into(),
    });
    let confirmation = tokio::spawn({
        let state = f.state.clone();
        let window = window.clone();
        let request = request.clone();
        async move { state.confirm_sudo_scope(&window, &request).await }
    });
    let prompt = popup(&f.state).await;
    approve(&f, &prompt, None).await.unwrap();
    confirmation.await.unwrap().unwrap();
    let probe = Arc::new(tokio::sync::Notify::new());
    let mut admitted = f
        .state
        .with_external_command_authority(Some((&window.entry_id, &request)));
    admitted.observe_app_lock_wait_for_test(probe.clone());
    admitted.authorize_current_external_command().unwrap();
    let app = f.app.lock().await;
    let before = app.config().user_config.workflow.session_default_max_agents;
    let task = tokio::spawn(async move {
        admitted
            .set_user_config_value("workflow.session_default_max_agents".into(), "16".into())
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    if expired {
        f.state
            .owned
            .sudo_turns
            .lock()
            .unwrap()
            .get_mut(&window.entry_id)
            .unwrap()
            .deadline = Some(std::time::Instant::now() - Duration::from_secs(1));
    } else {
        f.state
            .revoke_sudo(Some("local"), Some(&window.entry_id), "explicit_revoke")
            .unwrap();
    }
    drop(app);
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_err(),
        "a revoked command must not mutate configuration"
    );
    assert_eq!(
        f.app
            .lock()
            .await
            .config()
            .user_config
            .workflow
            .session_default_max_agents,
        before
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_timer_proof_exercises_the_scheduled_wake_path() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while f
            .state
            .owned
            .sudo_timers
            .lock()
            .unwrap()
            .get(&window.entry_id)
            != Some(&(1, 0))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.state
        .owned
        .sudo_timers
        .lock()
        .unwrap()
        .remove(&window.entry_id);
    tokio::time::pause();
    f.state.arm_sudo_timer(&window.entry_id, window.revision);
    tokio::task::yield_now().await;
    assert!(
        !f.state
            .owned
            .sudo_timers
            .lock()
            .unwrap()
            .contains_key(&window.entry_id),
        "proof must come from a scheduled verification tick, not task creation"
    );
    tokio::time::advance(Duration::from_millis(30)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        f.state
            .owned
            .sudo_timers
            .lock()
            .unwrap()
            .get(&window.entry_id),
        Some(&(1, 0))
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_window_starts_at_verification_without_banking_consumer_delay() {
    let f = fixture_with_options(None, true);
    let mut turn = running(&f);
    let verified = std::time::Instant::now() - Duration::from_secs(90);
    super::window::open_window_at(&mut turn, 60, (verified, 1_000));
    assert_eq!(turn.deadline, Some(verified + Duration::from_secs(3600)));
    assert_eq!(turn.expires_at_ms, Some(3_601_000));
    assert!(
        turn.deadline
            .unwrap()
            .saturating_duration_since(std::time::Instant::now())
            <= Duration::from_secs(3510)
    );
}

#[tokio::test]
async fn sudo_expiry_wake_retries_after_a_failed_inbox_write() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let task = window.task_id.clone().unwrap();
    let store = &f.state.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: "local".into(),
            room: window.session_id.clone(),
            agent: window.agent_id.clone(),
            prompt: task.clone(),
            run: None,
            id: "expiry-wait-source".into(),
            kind: "review".into(),
            resource: Some("review-source".into()),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Subscribe {
            task: task.clone(),
            prompt: task.clone(),
            registration: ledger::Registration {
                id: "expiry-reg".into(),
                task_id: task.clone(),
                source_id: "review-source".into(),
                obligation_id: Some("expiry-wait-source".into()),
                source_cursor: 0,
                live: true,
            },
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Yield {
            task: task.clone(),
            prompt: task.clone(),
            registrations: vec!["expiry-reg".into()],
            cursor: 0,
            deadline: now + 7_200_000,
            reason: "waiting for review".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: window.session_id.clone(),
            agent: window.agent_id.clone(),
            prompt: task.clone(),
            run: f.run.id().into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(
        store
            .agent_tasks(Some(&window.session_id), Some(&window.agent_id))
            .unwrap()[0]
            .state,
        ledger::ExecutionState::Waiting
    );
    let database = rusqlite::Connection::open(store.path()).unwrap();
    database.execute_batch("CREATE TRIGGER reject_sudo_expiry BEFORE INSERT ON agent_inbox BEGIN SELECT RAISE(ABORT, 'injected expiry wake failure'); END;").unwrap();
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .get_mut(&window.entry_id)
        .unwrap()
        .deadline = Some(std::time::Instant::now() - Duration::from_secs(1));
    f.state.pump_sudo_windows(None);
    assert!(
        f.state.list_sudo_turns("local").is_empty(),
        "authority must end even when the wake write fails"
    );
    assert!(f
        .state
        .owned
        .sudo_end_wakes
        .lock()
        .unwrap()
        .contains_key(&window.entry_id));
    database
        .execute_batch("DROP TRIGGER reject_sudo_expiry")
        .unwrap();
    f.state.pump_sudo_windows(None);
    assert!(f.state.owned.sudo_end_wakes.lock().unwrap().is_empty());
    let wakes = store
        .agent_inbox(&window.session_id, &window.agent_id, 0)
        .unwrap();
    assert_eq!(
        wakes
            .iter()
            .filter(|e| e.kind == "sudo_ended" && e.payload["task_id"] == task)
            .count(),
        1
    );
    f.state.recover_sudo_notices();
    assert_eq!(
        store
            .agent_inbox(&window.session_id, &window.agent_id, 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "sudo_ended")
            .count(),
        1,
        "restart replay must not duplicate the wake"
    );
}

#[test]
fn sudo_failed_admission_rollback_keeps_the_kernel_responsive() {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::state::sudo::window_tests::sudo_failed_admission_rollback_child",
            "--ignored",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "sudo rollback subprocess failed: {status}"
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            // This Child owns exactly the unreaped process it spawned; never
            // signal a group or special PID, including on a failed assertion.
            assert!(i32::try_from(child.id()).is_ok_and(|pid| pid > 1));
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("failed sudo admission deadlocked the kernel instead of returning an error");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated subprocess for the deadlock regression"]
async fn sudo_failed_admission_rollback_child() {
    let f = fixture_with_options(None, true);
    let database = rusqlite::Connection::open(f.state.owned.durable_state_store.path()).unwrap();
    database.execute_batch("CREATE TRIGGER reject_sudo_admission BEFORE INSERT ON agent_tasks BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;").unwrap();
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    approve(&f, &prompt, None).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(f.state.list_sudo_turns("local").is_empty());
    assert!(f
        .state
        .owned
        .session_snapshot(&f.request.session_id)
        .unwrap()
        .sudo_windows()
        .is_empty());
    assert!(f.state.owned.sudo_verified_at.lock().unwrap().is_empty());
}

// MP-08/MP-10/MP-11: a held queue head must not monopolize the App lock.
#[test]
fn sudo_deferred_app_queue_keeps_other_commands_responsive() {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::state::sudo::window_tests::sudo_deferred_app_queue_child",
            "--ignored",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "sudo deferred queue subprocess failed: {status}"
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            assert!(i32::try_from(child.id()).is_ok_and(|pid| pid > 1));
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("sudo deferred queue monopolized the App instead of returning pending");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolated subprocess for the deferred queue regression"]
async fn sudo_deferred_app_queue_child() {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session(&f), &window.agent_id)
        .unwrap();
    let unrelated = crate::app::KernelPreparedPromptSubmission {
        session_id: window.session_id.clone(),
        prompt: PromptQueueItem::new(
            "deferred-app-command",
            &f.request.attachment_id,
            &window.agent_id,
            "ordinary owner work",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    assert!(matches!(
        f.state
            .owned
            .submit_local_prepared_prompt_with_queue_policy(&unrelated, true)
            .unwrap()
            .unwrap()
            .outcome,
        PromptSubmissionOutcome::Queued { .. }
    ));
    let mut app = f.app.lock().await;
    assert!(app
        .advance_next_queued_prompt(&window.session_id, &window.agent_id)
        .unwrap()
        .is_none());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(&window.session_id, &window.agent_id)
            .unwrap(),
        1
    );
    drop(app);
    assert_eq!(f.state.list_sudo_turns("local").len(), 1);
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}
