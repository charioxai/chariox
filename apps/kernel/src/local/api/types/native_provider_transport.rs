use serde::{Deserialize, Serialize};

/// MP-08 / MP-10 / MP-11: a native display connection to the current owning run.
/// No caller-selected endpoint or prompt/permission execution is admitted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProviderTransportRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub agent_id: String,
    pub provider_run_id: String,
    pub operation: NativeProviderTransportOperation,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeProviderTransportOperation {
    Open,
    Exchange {
        connection_id: String,
        frames: Vec<String>,
    },
    Close {
        connection_id: String,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProviderTransportResult {
    pub connection_id: String,
    pub frames: Vec<String>,
    pub closed: bool,
}

// MP-11: official native frames may contain private provider configuration.
// Debug output reports bounds only, never frame contents or connection handles.
impl std::fmt::Debug for NativeProviderTransportOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => f.write_str("Open"),
            Self::Close { .. } => f.write_str("Close"),
            Self::Exchange { frames, .. } => f
                .debug_struct("Exchange")
                .field("frame_count", &frames.len())
                .finish_non_exhaustive(),
        }
    }
}
impl std::fmt::Debug for NativeProviderTransportResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeProviderTransportResult")
            .field("frame_count", &self.frames.len())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_native_transport_debug_withholds_private_frames_and_handles() {
        let operation = NativeProviderTransportOperation::Exchange {
            connection_id: "private-handle-canary".into(),
            frames: vec!["private-provider-config-canary".into()],
        };
        let result = NativeProviderTransportResult {
            connection_id: "private-handle-canary".into(),
            frames: vec!["private-provider-config-canary".into()],
            closed: false,
        };
        for debug in [format!("{operation:?}"), format!("{result:?}")] {
            assert!(!debug.contains("canary"));
            assert!(debug.contains("frame_count"));
        }
    }
}
