use serde::{Deserialize, Serialize};

const MAX_PROVIDER_TERMINATION_REASON_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRunTerminationCategory {
    ProcessExit,
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
        let mut sanitized = String::with_capacity(reason.len().min(256));
        let mut pending_space = false;
        for character in reason.chars() {
            if character.is_control() || character.is_whitespace() {
                pending_space = !sanitized.is_empty();
                continue;
            }
            if pending_space && sanitized.chars().count() < MAX_PROVIDER_TERMINATION_REASON_CHARS {
                sanitized.push(' ');
            }
            pending_space = false;
            if sanitized.chars().count() >= MAX_PROVIDER_TERMINATION_REASON_CHARS {
                break;
            }
            sanitized.push(character);
        }
        if sanitized.is_empty() {
            sanitized.push_str(fallback);
        }
        Self {
            category,
            reason: sanitized,
            timestamp_ms,
        }
    }
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
}
