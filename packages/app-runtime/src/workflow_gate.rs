//! Pure workflow-entry evaluation contract. Authority, persistence and routing
//! belong to the kernel; the worker can only return a bounded proposed decision.
mod json;
mod validation;
pub use json::{canonical_json, parse_json, sha256};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROFILE: &str = "gate_v1;wasmtime=50.0.0;fuel=10000000;stack=262144;epoch=5ms;execution=250ms;memory=16777216;tables=4096;instances=16;utf8=true;integer-only=true;component-async=false;memory-reservation=0;memory-guard-size=65536;memory-init-cow=false";
pub const ABI: &str = "gate_v1";
pub const FUEL: u64 = 10_000_000;
pub const INPUT_BYTES: usize = 512 * 1024;
pub const OUTPUT_BYTES: usize = 128 * 1024;
pub const CONFIG_BYTES: usize = 64 * 1024;
pub const STATE_BYTES: usize = 16 * 1024;
pub const CONTEXT_BYTES: usize = 64 * 1024;
pub const MAX_TARGETS: usize = 16;
pub const TOTAL_MS: u64 = 2000;
pub const EXECUTION_MS: u64 = 250;
pub const MAX_EVALUATIONS: u32 = 32;
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub type JsonObject = Map<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    #[error("gate_input_limit")]
    GateInputLimit,
    #[error("gate_invalid_input")]
    GateInvalidInput,
    #[error("gate_invalid_output")]
    GateInvalidOutput,
    #[error("gate_invalid_target")]
    GateInvalidTarget,
    #[error("gate_invalid_time")]
    GateInvalidTime,
    #[error("gate_limit")]
    GateLimit,
    #[error("gate_fuel_exhausted")]
    GateFuelExhausted,
    #[error("gate_trapped")]
    GateTrapped,
    #[error("gate_timeout")]
    GateTimeout,
    #[error("gate_resource_limit")]
    GateResourceLimit,
    #[error("gate_worker_failed")]
    GateWorkerFailed,
    #[error("gate_runtime_unavailable")]
    GateRuntimeUnavailable,
    #[error("gate_interrupted")]
    GateInterrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub workflow_revision: String,
    pub config_sha256: String,
    pub config: JsonObject,
}
impl Configuration {
    pub fn new(workflow_revision: String, config: JsonObject) -> Result<Self, Failure> {
        let bytes = canonical_json(&Value::Object(config.clone()), CONFIG_BYTES)?;
        Ok(Self {
            workflow_revision,
            config_sha256: sha256(&bytes),
            config,
        })
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub workflow: JsonObject,
    pub binding: JsonObject,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Trigger,
    Resume,
    BatchFlush,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    AppEvent,
    WorkflowNotification,
    Http,
    Schedule,
    Manual,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSchema {
    pub source_kind: String,
    pub event_type: String,
    pub version: u32,
    pub sha256: String,
    pub schema: JsonObject,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub occurrence_id: String,
    pub occurred_at_ms: Option<u64>,
    pub data: Option<JsonObject>,
    pub data_schema: Option<DataSchema>,
    pub prompt: Option<String>,
    // Descriptive metadata only. No attachment or file-opening authority.
    pub artifacts: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchItem {
    pub entry_id: String,
    pub payload: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub key: String,
    pub flush_at_ms: u64,
    pub items: Vec<BatchItem>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub abi: String,
    pub entry_id: String,
    pub endpoint_id: String,
    pub allowed_entry_nodes: Vec<String>,
    pub binding_id: String,
    pub phase: Phase,
    pub accepted_at_ms: u64,
    pub evaluation_at_ms: u64,
    pub deadline_ms: u64,
    pub trigger: Option<Trigger>,
    pub batch: Option<Batch>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evaluation {
    pub input: Input,
    pub configuration: Configuration,
    pub state: State,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub node_id: String,
    pub payload: Value,
    pub prompt: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Decision {
    Run {
        targets: Vec<Route>,
    },
    Drop,
    DeferUntil {
        at_ms: u64,
    },
    Batch {
        key: String,
        flush_at_ms: u64,
        payload: Value,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub decision: Decision,
    pub state: State,
    pub reason: String,
    pub logs: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub outcome: Result<Output, Failure>,
    pub fuel_consumed: u64,
    pub execution_ms: u64,
    pub profile: String,
}

/// Supervisor-only launch identity; never an App installation or broker grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub validate_only: bool,
    pub generation: String,
    pub entry_id: String,
    pub module_sha256: String,
    pub evaluation: Evaluation,
}
impl Launch {
    pub fn new(validate_only: bool, component: &[u8], evaluation: Evaluation) -> Self {
        Self {
            validate_only,
            // The shared native worker record uses a positive signed integer.
            generation: (rand::random::<u64>() % i64::MAX as u64 + 1).to_string(),
            entry_id: evaluation.input.entry_id.clone(),
            module_sha256: sha256(component),
            evaluation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub generation: String,
    pub entry_id: String,
    pub module_sha256: String,
    pub report: Report,
}

/// Portable workflow setup. Mutable state is deliberately not accepted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub abi: String,
    pub module_sha256: String,
    pub config: JsonObject,
    pub config_sha256: String,
}
impl Definition {
    pub fn validate(&self) -> Result<(), Failure> {
        if self.abi != ABI
            || self.module_sha256.len() != 71
            || !self.module_sha256.starts_with("sha256:")
            || !self.module_sha256[7..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(Failure::GateInvalidInput);
        }
        if sha256(&canonical_json(&self.config.clone().into(), CONFIG_BYTES)?) != self.config_sha256
        {
            return Err(Failure::GateInvalidInput);
        }
        Ok(())
    }
}
