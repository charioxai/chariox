//! MP-08 / MP-10 / MP-11: exact worker turn outcomes, including native-origin turns.
use crate::{
    app::DaemonApp, execution_lease::LeasedAgent, terminal::AssistantMessageCompletionRecord,
    transport::relay_peer::RelayProjectedCompletion,
};

// Private worker receipts bind ordinary turns as precisely as workflow turns.
// No field is added to LocalDaemonResponse or the relay protocol.
pub(super) fn backing_prompt_id<'a>(
    app: &'a DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    home: &str,
) -> Option<&'a str> {
    let record = app.worker_prompt_receipts.get(&leased.id, home)?;
    (record.receipt.execution_lease_id == leased.lease_id
        && record.receipt.phase
            == crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase::Accepted
        && record.receipt.worker_provider_run_id.as_deref() == Some(run))
    .then(|| record.receipt.backing_prompt_id.as_deref())
    .flatten()
}

pub(super) fn project(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    home_prompt: Option<&str>,
    backing_prompt: Option<&str>,
    record: AssistantMessageCompletionRecord,
) -> Option<RelayProjectedCompletion> {
    let prompt = record.prompt_id.as_deref().or(backing_prompt);
    let active_backing_prompt = app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased.backing_session_id,
            &leased.backing_agent_id,
        )
        .ok()
        .flatten();
    let exact_settled_receipt = prompt.is_some_and(|id| {
        app.worker_prompt_receipts
            .home_prompt_for_backing(&leased.id, &leased.lease_id, run, id)
            .is_some()
            && app
                .completed_git_turn_snapshot_store()
                .projection_for_prompt(&leased.backing_session_id, &leased.backing_agent_id, id)
                .is_some_and(|turn| turn.provider_run_id == run)
    });
    if !exact_settled_receipt
        && leased.active_home_prompt_id.is_some()
        && record.prompt_id.is_some()
        && active_backing_prompt
            .as_ref()
            .is_some_and(|active| prompt != Some(active.id()))
    {
        return None; // An old message cannot settle a successor's managed turn.
    }
    let binding = prompt.and_then(|id| {
        app.leased_workflow_turns.values().find(|binding| {
            binding.leased_agent_id == leased.id
                && binding.provider_run_id == run
                && binding.backing_prompt_id == id
        })
    });
    let mut completion = RelayProjectedCompletion {
        message_id: record.message_id,
        completed_at_ms: record.completed_at_ms,
        home_prompt_id: binding
            .map(|binding| binding.home_prompt_id.clone())
            .or_else(|| {
                prompt.and_then(|backing| {
                    app.worker_prompt_receipts
                        .home_prompt_for_backing(&leased.id, &leased.lease_id, run, backing)
                        .map(str::to_string)
                })
            })
            .or_else(|| {
                if leased.active_home_prompt_id.is_some() {
                    home_prompt.map(str::to_string)
                } else {
                    record
                        .prompt_id
                        .clone()
                        .or_else(|| home_prompt.map(str::to_string))
                }
            }),
        settlement_status: None,
        provider_termination: None,
    };
    if let Some(turn) = prompt
        .and_then(|id| {
            app.completed_git_turn_snapshot_store()
                .projection_for_prompt(&leased.backing_session_id, &leased.backing_agent_id, id)
        })
        .filter(|turn| turn.provider_run_id == run)
    {
        completion.settlement_status = Some(turn.settlement_status);
        completion.provider_termination = turn.provider_termination;
    }
    Some(completion)
}

pub(super) fn refresh(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    completion: &mut RelayProjectedCompletion,
) {
    let Some(home_prompt) = completion.home_prompt_id.as_deref() else {
        return;
    };
    let backing_prompt = app
        .leased_workflow_turns
        .values()
        .find(|binding| {
            binding.leased_agent_id == leased.id
                && binding.provider_run_id == run
                && binding.home_prompt_id == home_prompt
        })
        .map(|binding| binding.backing_prompt_id.as_str())
        .or_else(|| backing_prompt_id(app, leased, run, home_prompt))
        .unwrap_or(home_prompt);
    if let Some(turn) = app
        .completed_git_turn_snapshot_store()
        .projection_for_prompt(
            &leased.backing_session_id,
            &leased.backing_agent_id,
            backing_prompt,
        )
        .filter(|turn| turn.provider_run_id == run)
    {
        completion.settlement_status = Some(turn.settlement_status);
        completion.provider_termination = turn.provider_termination;
    }
}

// MP-08 / MP-10 / MP-11: deduplicate a privately bound turn across provider,
// generic prompt and recovered settlement records, retaining provider IDs on wire.
pub(super) fn completion_key(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    message: &str,
    home: Option<&str>,
) -> String {
    if let Some(backing) = home.and_then(|home| backing_prompt_id(app, leased, run, home)) {
        return format!(
            "{}:{}:{}:{run}:settled:{backing}",
            leased.backing_session_id, leased.lease_id, leased.id
        );
    }
    super::leased_provider_run_completion_key(leased, run, message)
}

// The prompt owner is the authority even when a message was drained before its
// acknowledgement. Recover only an exact settled backing turn, never idle/output.
pub(super) fn append_settled_receipts(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    completions: &mut Vec<RelayProjectedCompletion>,
) {
    completions.retain(|completion| {
        completion.settlement_status.is_some()
            || completion
                .home_prompt_id
                .as_deref()
                .and_then(|home| backing_prompt_id(app, leased, run, home))
                .is_none()
    });
    let mut keys = completions
        .iter()
        .map(|completion| {
            completion_key(
                app,
                leased,
                run,
                &completion.message_id,
                completion.home_prompt_id.as_deref(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    for (home, backing) in
        app.worker_prompt_receipts
            .backing_bindings_for_run(&leased.id, &leased.lease_id, run)
    {
        let Some(turn) = app
            .completed_git_turn_snapshot_store()
            .projection_for_prompt(
                &leased.backing_session_id,
                &leased.backing_agent_id,
                &backing,
            )
            .filter(|turn| turn.provider_run_id == run)
        else {
            continue;
        };
        let message_id = format!("leased-{run}-settled:{backing}");
        let key = completion_key(app, leased, run, &message_id, Some(&home));
        if leased.projected_completion_keys.contains(&key) || !keys.insert(key) {
            continue;
        }
        completions.push(RelayProjectedCompletion {
            message_id,
            completed_at_ms: turn.completed_at_ms,
            home_prompt_id: Some(home),
            settlement_status: Some(turn.settlement_status),
            provider_termination: turn.provider_termination,
        });
    }
    // Message records and recovered proofs may arrive in the same drain. Home
    // must observe a predecessor before its already-settled queued successor.
    completions.sort_by_key(|completion| completion.completed_at_ms);
}
