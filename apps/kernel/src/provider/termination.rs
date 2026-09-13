use serde::{Deserialize, Serialize};

const MAX_PROVIDER_TERMINATION_REASON_CHARS: usize = 256;
const MAX_PROVIDER_TERMINATION_SIGNAL_CHARS: usize = 64;
const REDACTED_DIAGNOSTIC_VALUE: &str = "[redacted]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRunTerminationCategory {
    ProcessExit,
    Signal,
    ExplicitProviderError,
    RuntimeFailure,
    TransportFailure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunTermination {
    pub category: ProviderRunTerminationCategory,
    pub reason: String,
    pub timestamp_ms: u64,
}

impl ProviderRunTermination {
    pub fn process_exit(exit_code: u32, timestamp_ms: u64) -> Self {
        Self {
            category: ProviderRunTerminationCategory::ProcessExit,
            reason: format!("provider process exited with status {exit_code}"),
            timestamp_ms,
        }
    }

    pub fn signal(signal_name: &str, timestamp_ms: u64) -> Self {
        let signal_name = sanitize_signal_name(signal_name).unwrap_or("unknown");
        Self {
            category: ProviderRunTerminationCategory::Signal,
            reason: format!("provider process terminated by signal {signal_name}"),
            timestamp_ms,
        }
    }

    pub fn explicit_provider_error(reason: &str, timestamp_ms: u64) -> Self {
        Self::bounded(
            ProviderRunTerminationCategory::ExplicitProviderError,
            reason,
            "provider reported an explicit error",
            timestamp_ms,
        )
    }

    pub fn unknown_process_exit(timestamp_ms: u64) -> Self {
        Self {
            category: ProviderRunTerminationCategory::Unknown,
            reason: "provider process exited without an available status".to_string(),
            timestamp_ms,
        }
    }

    pub fn runtime_failure(reason: &str, timestamp_ms: u64) -> Self {
        Self::bounded(
            ProviderRunTerminationCategory::RuntimeFailure,
            reason,
            "provider runtime failed",
            timestamp_ms,
        )
    }

    pub fn transport_failure(reason: &str, timestamp_ms: u64) -> Self {
        Self::bounded(
            ProviderRunTerminationCategory::TransportFailure,
            reason,
            "provider transport failed",
            timestamp_ms,
        )
    }

    fn bounded(
        category: ProviderRunTerminationCategory,
        reason: &str,
        fallback: &str,
        timestamp_ms: u64,
    ) -> Self {
        let sanitized = sanitize_provider_diagnostic(reason);
        let sanitized = sanitized
            .chars()
            .take(MAX_PROVIDER_TERMINATION_REASON_CHARS)
            .collect::<String>();
        let sanitized = if sanitized.is_empty() {
            fallback.to_string()
        } else {
            sanitized
        };
        Self {
            category,
            reason: sanitized,
            timestamp_ms,
        }
    }
}

pub(crate) fn sanitize_provider_diagnostic(reason: &str) -> String {
    let normalized = reason
        .chars()
        .map(|character| {
            if character.is_control() || character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut sanitized = String::new();
    let mut redact_until_assignment = false;
    let mut redact_next = false;

    for token in normalized.split_whitespace() {
        if redact_until_assignment {
            if let Some((key, _, _)) = assignment_parts(token) {
                if is_sensitive_key(key) {
                    continue;
                }
                redact_until_assignment = false;
            } else {
                continue;
            }
        }

        if redact_next {
            if is_bearer_scheme(token) {
                push_diagnostic_token(&mut sanitized, token);
                continue;
            }
            push_diagnostic_token(&mut sanitized, REDACTED_DIAGNOSTIC_VALUE);
            redact_next = false;
            continue;
        }

        if let Some((key, separator, value)) = assignment_parts(token) {
            if is_sensitive_key(key) {
                push_diagnostic_token(
                    &mut sanitized,
                    &format!(
                        "{}{}{}",
                        sanitize_key(key),
                        separator,
                        REDACTED_DIAGNOSTIC_VALUE
                    ),
                );
                if value.is_empty() || sensitive_value_can_contain_spaces(key) {
                    redact_until_assignment = true;
                }
                continue;
            }
        }

        if is_bearer_scheme(token) {
            push_diagnostic_token(&mut sanitized, token);
            redact_next = true;
            continue;
        }

        if token_contains_sensitive_assignment(token) {
            push_diagnostic_token(&mut sanitized, REDACTED_DIAGNOSTIC_VALUE);
            continue;
        }

        if contains_json_sensitive_field(token) || looks_like_secret_token(token) {
            push_diagnostic_token(&mut sanitized, REDACTED_DIAGNOSTIC_VALUE);
            continue;
        }
        push_diagnostic_token(&mut sanitized, token);
    }

    sanitized
        .chars()
        .take(MAX_PROVIDER_TERMINATION_REASON_CHARS)
        .collect()
}

fn sanitize_signal_name(signal_name: &str) -> Option<&str> {
    let signal_name = signal_name.trim();
    if signal_name.is_empty()
        || signal_name.chars().count() > MAX_PROVIDER_TERMINATION_SIGNAL_CHARS
        || !signal_name.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '-' | '_' | '(' | ')' | '/')
        })
    {
        return None;
    }
    Some(signal_name)
}

fn assignment_parts(token: &str) -> Option<(&str, char, &str)> {
    let (index, separator) = token.char_indices().find_map(|(index, character)| {
        matches!(character, '=' | ':').then_some((index, character))
    })?;
    let key = token[..index].trim_matches(|character: char| {
        matches!(character, '"' | '\'' | '{' | '[' | '(' | ',' | ';')
    });
    (!key.is_empty()).then_some((key, separator, &token[index + separator.len_utf8()..]))
}

fn sanitize_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .to_ascii_lowercase()
        .replace('-', "_");
    key.contains("authorization")
        || key.contains("api_key")
        || key.contains("apikey")
        || key == "token"
        || key.ends_with("_token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("cookie")
        || key.contains("credential")
        || key.contains("prompt")
        || key.contains("command")
        || key == "cmd"
        || key == "env"
        || key == "stdin"
        || key == "stdout"
        || key == "stderr"
        || key.contains("tool");
}

fn sensitive_value_can_contain_spaces(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("authorization")
        || key.contains("prompt")
        || key.contains("command")
        || key == "cmd"
        || key == "env"
        || key == "stdin"
        || key == "stdout"
        || key == "stderr"
        || key.contains("tool")
}

fn is_bearer_scheme(token: &str) -> bool {
    matches!(
        token
            .trim_matches(|character: char| !character.is_ascii_alphabetic())
            .to_ascii_lowercase()
            .as_str(),
        "bearer" | "basic"
    )
}

fn token_contains_sensitive_assignment(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    [
        "authorization=",
        "authorization:",
        "api_key=",
        "apikey=",
        "access_token=",
        "refresh_token=",
        "token=",
        "password=",
        "secret=",
        "cookie=",
        "prompt=",
        "command=",
        "cmd=",
        "env=",
        "stdin=",
        "stdout=",
        "stderr=",
        "tool=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn contains_json_sensitive_field(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    [
        "\"authorization\":",
        "\"api_key\":",
        "\"apikey\":",
        "\"access_token\":",
        "\"refresh_token\":",
        "\"token\":",
        "\"password\":",
        "\"secret\":",
        "\"cookie\":",
        "\"prompt\":",
        "\"command\":",
        "\"env\":",
        "\"stdin\":",
        "\"stdout\":",
        "\"stderr\":",
        "\"tool\":",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn looks_like_secret_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    (lower.starts_with("sk-")
        || lower.starts_with("sk_")
        || lower.starts_with("ghp_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("eyj"))
        && token.chars().count() >= 12
}

fn push_diagnostic_token(output: &mut String, token: &str) {
    let token = token
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    *character,
                    '-' | '_' | '.' | '/' | ':' | ',' | ';' | '(' | ')' | '[' | ']' | '\'' | '`'
                )
        })
        .collect::<String>();
    if token.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push(' ');
    }
    output.push_str(&token);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_termination_reason_is_sanitized_and_bounded() {
        let input = format!("  relay\nclosed\0{}", "x".repeat(400));
        let termination = ProviderRunTermination::transport_failure(&input, 42);

        assert_eq!(
            termination.category,
            ProviderRunTerminationCategory::TransportFailure,
        );
        assert!(!termination.reason.chars().any(char::is_control));
        assert!(termination.reason.starts_with("relay closed"));
        assert_eq!(termination.reason.chars().count(), 256);
        assert_eq!(termination.timestamp_ms, 42);
    }

    #[test]
    fn empty_provider_termination_reason_uses_category_fallback() {
        assert_eq!(
            ProviderRunTermination::runtime_failure("\n\0", 1).reason,
            "provider runtime failed",
        );
    }

    #[test]
    fn process_signal_and_explicit_error_are_distinguishable() {
        let signal = ProviderRunTermination::signal("SIGTERM", 7);
        assert_eq!(signal.category, ProviderRunTerminationCategory::Signal);
        assert_eq!(
            signal.reason,
            "provider process terminated by signal SIGTERM"
        );

        let explicit = ProviderRunTermination::explicit_provider_error(
            "provider rejected request stderr=raw-stderr api_key=sk-live-secret prompt=private prompt command=rm -rf",
            8,
        );
        assert_eq!(
            explicit.category,
            ProviderRunTerminationCategory::ExplicitProviderError,
        );
        assert_eq!(explicit.timestamp_ms, 8);
        assert!(explicit.reason.contains("provider rejected request"));
        assert!(explicit.reason.contains("[redacted]"));
        assert!(!explicit.reason.contains("raw-stderr"));
        assert!(!explicit.reason.contains("sk-live-secret"));
        assert!(!explicit.reason.contains("private prompt"));
        assert!(!explicit.reason.contains("rm -rf"));
    }

    #[test]
    fn empty_and_large_secret_bearing_evidence_stays_bounded() {
        let empty = ProviderRunTermination::explicit_provider_error("\n\0", 9);
        assert_eq!(empty.reason, "provider reported an explicit error");

        let unknown = ProviderRunTermination::unknown_process_exit(9);
        assert_eq!(unknown.category, ProviderRunTerminationCategory::Unknown);
        assert!(unknown.reason.contains("without an available status"));

        let large = ProviderRunTermination::explicit_provider_error(
            &format!(
                "authorization: Bearer {} stderr={} {}",
                "secret-token".repeat(20),
                "prompt-body".repeat(20),
                "x".repeat(500),
            ),
            10,
        );
        assert!(large.reason.chars().count() <= MAX_PROVIDER_TERMINATION_REASON_CHARS);
        assert!(!large.reason.contains("secret-token"));
        assert!(!large.reason.contains("prompt-body"));
    }
}
