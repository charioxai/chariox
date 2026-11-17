//! The prompt a responsible agent receives for one occurrence of its workflow's "run finished"
//! source. The kernel writes it from the typed fields of the occurrence; the text a run produced
//! (its input, its output) never enters it.

fn failure_words(reason: &str) -> &'static str {
    match reason {
        "step_needs_help" => "a step needs help",
        "provider_failure" => "a provider failed",
        "transport_failure" => "a connection failed",
        "turn_stalled" => "a step stalled",
        "output_validation_failed" => "a step's output was not accepted",
        "workflow_run_output_validation_failed" => "the run's output was not accepted",
        "node_turn_budget_exhausted" => "a step used all its turns",
        "missing_ack" | "missing_structured_output" => "a step did not report back",
        "run_stopped" => "the run was stopped",
        _ => "the run failed",
    }
}

fn waiting_words(on: &str) -> &'static str {
    match on {
        "provider_profile_usage_exhausted" => "a provider usage limit",
        "provider_profile_unauthenticated" => "a provider sign-in",
        "provider_profile_unavailable" | "provider_profile_unverified" => "a provider",
        "blocked_on_workspace_claim" => "a busy workspace",
        _ => "something",
    }
}

/// `trailer` is the fixed closing of the responsible's prompt; `envelope` is the notification
/// envelope of one occurrence.
pub(super) fn responsible_prompt(trailer: &str, envelope: &serde_json::Value) -> String {
    let body: serde_json::Value = envelope["output"]["message"]
        .as_str()
        .and_then(|message| serde_json::from_str(message).ok())
        .unwrap_or(serde_json::Value::Null);
    let name = body["workflow_name"].as_str().unwrap_or("a workflow");
    let run = match body["number"].as_u64() {
        Some(number) if number > 0 => format!("Run #{number}"),
        _ => "A run".to_owned(),
    };
    let outcome = &body["outcome"];
    match outcome["kind"].as_str() {
        Some("failed") => {
            let reason = outcome["reason"].as_str().unwrap_or_default();
            let detail = outcome["detail"]
                .as_str()
                .filter(|detail| !detail.is_empty())
                .map(|detail| format!(": {detail}"))
                .unwrap_or_default();
            format!(
                "{name} · {run} failed — {}{detail}. {trailer}",
                failure_words(reason)
            )
        }
        Some("waiting") => format!(
            "{name} · {run} is waiting on {}. {trailer}",
            waiting_words(outcome["on"].as_str().unwrap_or_default())
        ),
        _ => format!("{name} · {run} {}.", completed_words(&body)),
    }
}

fn completed_words(body: &serde_json::Value) -> &'static str {
    match body["status"].as_str() {
        Some("Completed") => "completed",
        _ => "ended",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAILER: &str = "You are responsible for this workflow.";

    fn envelope(body: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"output": {"message": body.to_string()}})
    }

    #[test]
    fn a_failed_run_reads_as_the_kernels_own_sentence() {
        let text = responsible_prompt(
            TRAILER,
            &envelope(serde_json::json!({
                "workflow_name": "release-pipeline",
                "number": 3,
                "status": "Failed",
                "prompt": "ship it",
                "failure": "resend the diff",
                "outcome": {"kind": "failed", "reason": "step_needs_help", "detail": "resend the diff"},
            })),
        );
        assert_eq!(
            text,
            "release-pipeline · Run #3 failed — a step needs help: resend the diff. You are responsible for this workflow."
        );
        assert!(!text.contains("ship it"), "the run's input never enters it");
    }

    #[test]
    fn a_waiting_run_names_the_thing_it_waits_on() {
        let text = responsible_prompt(
            TRAILER,
            &envelope(serde_json::json!({
                "workflow_name": "release-pipeline",
                "number": 4,
                "status": "Waiting",
                "outcome": {"kind": "waiting", "on": "provider_profile_usage_exhausted", "node_id": "n"},
            })),
        );
        assert_eq!(
            text,
            "release-pipeline · Run #4 is waiting on a provider usage limit. You are responsible for this workflow."
        );
    }

    #[test]
    fn a_completed_run_needs_nothing() {
        let text = responsible_prompt(
            TRAILER,
            &envelope(serde_json::json!({
                "workflow_name": "release-pipeline",
                "number": 5,
                "status": "Completed",
            })),
        );
        assert_eq!(text, "release-pipeline · Run #5 completed.");
    }
}
