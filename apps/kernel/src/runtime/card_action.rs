//! Person cards: an action an agent asks the person to authorize is parsed and described
//! by the kernel. The card shows this description, never the agent's own wording.
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;

const VALUE_CHARS: usize = 160;
const SUMMARY_CHARS: usize = 900;

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_request_popup",
        message: message.into(),
    }
}

/// The one request a card carries. What `/sudo` keeps for the person's own terminal stays
/// theirs here too: a card can authorize work, never approvals, access or secrets.
pub(crate) fn parse_card_action(
    request: serde_json::Value,
) -> Result<LocalDaemonRequest, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(request).map_err(|_| {
        refused("action.request must be one externally tagged kernel request, for example {\"AliasSession\":{\"session_id\":\"SESSION_ID\",\"alias\":\"new-name\"}}")
    })?;
    if crate::runtime::state::sudo_request_forbidden(&request) {
        return Err(refused(
            "a card cannot authorize this request: approvals, access, secrets, the passkey and pairing stay with the person's own terminal",
        ));
    }
    Ok(request)
}

fn shorten(text: &str, limit: usize) -> String {
    let mut shown: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        shown.push('…');
    }
    shown
}

fn render(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => shorten(text, VALUE_CHARS),
        other => shorten(&other.to_string(), VALUE_CHARS),
    }
}

/// What will run, in the kernel's words: the request's name and its values.
pub(crate) fn card_action_summary(agent_ref: &str, request: &LocalDaemonRequest) -> String {
    let value = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
    let (name, fields) = match &value {
        serde_json::Value::String(name) => (name.clone(), serde_json::Value::Null),
        serde_json::Value::Object(map) if map.len() == 1 => {
            let (name, fields) = map.iter().next().expect("one entry");
            (name.clone(), fields.clone())
        }
        _ => ("request".to_owned(), value.clone()),
    };
    let details = match &fields {
        serde_json::Value::Object(map) if !map.is_empty() => map
            .iter()
            .map(|(key, value)| format!("{key}: {}", render(value)))
            .collect::<Vec<_>>()
            .join(", "),
        serde_json::Value::Null => String::new(),
        other => render(other),
    };
    let text = if details.is_empty() {
        format!("{agent_ref} asks to run {name}.")
    } else {
        format!("{agent_ref} asks to run {name} ({details}).")
    };
    crate::secret_redaction::redact_secrets(&shorten(&text, SUMMARY_CHARS)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_the_request_and_its_values() {
        let request = parse_card_action(
            serde_json::json!({"AliasSession":{"session_id":"s-1","alias":"nightly"}}),
        )
        .unwrap();
        let summary = card_action_summary("builder", &request);
        assert_eq!(
            summary,
            "builder asks to run AliasSession (alias: nightly, session_id: s-1)."
        );
    }

    #[test]
    fn a_card_cannot_carry_what_stays_with_the_persons_terminal() {
        for forbidden in [
            serde_json::json!({"ListCredentials": null}),
            serde_json::json!({"RespondToInteraction":{"session_id":"s","interaction_id":"i","choice_id":"c"}}),
        ] {
            let error = parse_card_action(forbidden).unwrap_err().to_string();
            assert!(error.contains("cannot authorize"), "{error}");
        }
        assert!(parse_card_action(serde_json::json!({"NoSuchRequest":1})).is_err());
    }

    #[test]
    fn long_values_are_shortened() {
        let request = parse_card_action(
            serde_json::json!({"AliasSession":{"session_id":"s","alias":"x".repeat(1000)}}),
        )
        .unwrap();
        assert!(card_action_summary("a", &request).chars().count() < 400);
    }
}
