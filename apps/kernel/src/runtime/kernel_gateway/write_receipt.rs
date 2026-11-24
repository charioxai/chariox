//! The gateway acknowledges writes with changed rows and their Room IDs.
//! Room state and prompt content are read separately.

use serde_json::Value;

pub(crate) fn compact(response: &mut Value) {
    // A LocalDaemonResponse has exactly one top-level tag. Its top-level
    // session fields are RuntimeSession values, even when the operation's
    // changed row has its own nested session-related fields.
    let Some((kind, Value::Object(fields))) = response
        .as_object_mut()
        .and_then(|tagged| tagged.iter_mut().next())
    else {
        return;
    };
    if let Some(mut session) = fields.remove("session") {
        fields.insert("session_id".into(), session["id"].take());
        fields.remove("agent_activity");
        fields.remove("agent_activity_revision");
    }
    for (rooms, ids) in [
        ("sessions", "session_ids"),
        ("deleted_sessions", "deleted_session_ids"),
    ] {
        if let Some(Value::Array(sessions)) = fields.remove(rooms) {
            fields.insert(
                ids.into(),
                Value::Array(
                    sessions
                        .into_iter()
                        .map(|mut session| session["id"].take())
                        .collect(),
                ),
            );
        }
    }
    match kind.as_str() {
        "PromptSubmitted" => compact_submission(fields.get_mut("outcome")),
        "PromptsSubmitted" => {
            if let Some(Value::Array(results)) = fields.get_mut("results") {
                for result in results {
                    compact_submission(result.get_mut("outcome"));
                }
            }
        }
        "ActivePromptSteered"
        | "QueuedPromptSteered"
        | "QueuedPromptCancelled"
        | "QueuedPromptUpdated" => compact_prompt(fields.get_mut("prompt")),
        "PromptCompleted" => {
            if let Some(Value::Object(completion)) = fields.get_mut("completion") {
                compact_prompt(completion.get_mut("completed"));
                compact_prompt(completion.get_mut("started_next"));
            }
        }
        "PromptCancelled" => {
            if let Some(Value::Object(cancellation)) = fields.get_mut("cancellation") {
                compact_prompt(cancellation.get_mut("prompt"));
                compact_prompt(cancellation.get_mut("started_next"));
            }
        }
        _ => {}
    }
}

fn compact_submission(outcome: Option<&mut Value>) {
    if let Some(Value::Object(fields)) = outcome
        .and_then(Value::as_object_mut)
        .and_then(|variants| variants.values_mut().next())
    {
        compact_prompt(fields.get_mut("prompt"));
    }
}

fn compact_prompt(prompt: Option<&mut Value>) {
    if let Some(Value::Object(prompt)) = prompt {
        prompt.remove("prompt");
        prompt.remove("attachments");
    }
}
