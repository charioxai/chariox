//! Exact WIT records with bounded canonical lifting before guest post-return.
use super::bounded::{List, Text};
use wasmtime::component::{ComponentType, Lift, Lower};

#[derive(ComponentType, Lower)]
#[component(record)]
pub(super) struct Input {
    #[component(name = "entry-json")]
    pub entry_json: String,
}
#[derive(ComponentType, Lower)]
#[component(record)]
pub(super) struct Configuration {
    #[component(name = "workflow-revision")]
    pub workflow_revision: String,
    #[component(name = "config-sha256")]
    pub config_sha256: String,
    #[component(name = "config-json")]
    pub config_json: String,
}
#[derive(ComponentType, Lower)]
#[component(record)]
pub(super) struct StateInput {
    #[component(name = "workflow-json")]
    pub workflow_json: String,
    #[component(name = "binding-json")]
    pub binding_json: String,
}
#[derive(ComponentType, Lift)]
#[component(record)]
pub(super) struct State {
    #[component(name = "workflow-json")]
    pub workflow_json: Text<16384>,
    #[component(name = "binding-json")]
    pub binding_json: Text<16384>,
}
#[derive(ComponentType, Lift)]
#[component(record)]
pub(super) struct Route {
    #[component(name = "node-id")]
    pub node_id: Text<128>,
    #[component(name = "payload-json")]
    pub payload_json: Text<65536>,
    pub prompt: Option<Text<65536>>,
}
#[derive(ComponentType, Lift)]
#[component(record)]
pub(super) struct BatchValue {
    pub key: Text<128>,
    #[component(name = "flush-at-ms")]
    pub flush_at_ms: u64,
    #[component(name = "payload-json")]
    pub payload_json: Text<65536>,
}
#[derive(ComponentType, Lift)]
#[component(variant)]
pub(super) enum Decision {
    #[component(name = "run")]
    Run(List<Route, 16>),
    #[component(name = "drop")]
    Drop,
    #[component(name = "defer-until")]
    DeferUntil(u64),
    #[component(name = "batch")]
    Batch(BatchValue),
}
#[derive(ComponentType, Lift)]
#[component(record)]
pub(super) struct Output {
    pub decision: Decision,
    pub state: State,
    pub reason: Text<512>,
    pub logs: List<Text<256>, 32>,
}
