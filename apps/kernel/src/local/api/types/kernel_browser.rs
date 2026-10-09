//! MD-2: sessionless host browser protocol (legacy 417; display 419).
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserCommand {
    Computer {
        command: KernelComputerCommand,
    },
    /// MP-08 / MP-11 (local 461): explicit `null` restores every owned agent after
    /// "revoke all"; the field is required so an omitted agent cannot restore everyone.
    GrantRoomComputer {
        #[serde(deserialize_with = "Option::deserialize")]
        agent_id: Option<String>,
    },
    ListGrants,
    SubscribeGrants {
        after: u64,
        wait_ms: u32,
    },
    RevokeGrants {
        agent_id: Option<String>,
    },
    MirrorSubscribe {
        tab_id: String,
        generation: u64,
        device_scale_factor: u32,
    },
    MirrorNext {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
        drift_nodes: Vec<String>,
    },
    MirrorClose {
        subscription_id: String,
        generation: u64,
    },
    MirrorInput {
        tab_id: String,
        generation: u64,
        document_id: String,
        subscription_id: String,
        sequence: u64,
        action: KernelBrowserMirrorAction,
    },
    DisplayCapture {
        tab_id: String,
        generation: u64,
    },
    DisplayTakeover {
        tab_id: String,
        generation: u64,
    },
    DisplayRelease {
        tab_id: String,
        generation: u64,
    },
    DisplayActors,
    DisplaySubscribe {
        tab_id: String,
        generation: u64,
        codecs: Vec<String>,
        bitrate: u32,
        device_scale_factor: u32,
    },
    DisplayNext {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
    },
    /// MP-08/MP-10: protocol 475 pushed display. Acknowledges the newest
    /// presented sequence on a relay display subscription (the first ack
    /// starts the kernel push pump); `lost` requests an independent frame.
    DisplayAck {
        subscription_id: String,
        generation: u64,
        sequence: u64,
        lost: bool,
    },
    DisplayInput {
        tab_id: String,
        generation: u64,
        document_id: String,
        input: KernelBrowserInput,
    },
    Start,
    State,
    Stop,
    Open {
        url: String,
    },
    Close {
        tab_id: String,
        generation: u64,
    },
    Navigate {
        tab_id: String,
        generation: u64,
        url: String,
    },
    Snapshot {
        tab_id: String,
        generation: u64,
    },
    Input {
        tab_id: String,
        generation: u64,
        input: KernelBrowserInput,
    },
    Screenshot {
        tab_id: String,
        generation: u64,
    },
    Subscribe {
        tab_id: String,
        generation: u64,
    },
    Poll {
        subscription_id: String,
        generation: u64,
    },
    Unsubscribe {
        subscription_id: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserInput {
    Click {
        x: u32,
        y: u32,
    },
    Text {
        text: String,
    },
    Key {
        key: String,
    },
    Scroll {
        x: u32,
        y: u32,
        delta_x: i32,
        delta_y: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelBrowserRequest {
    pub command: KernelBrowserCommand,
}

/// MP-08/MP-11: element IDs are scoped to a caller-bound sanitized document epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserMirrorAction {
    Click {
        node_id: String,
    },
    Focus {
        node_id: String,
    },
    Text {
        node_id: String,
        text: String,
    },
    Scroll {
        node_id: String,
        delta_x: i32,
        delta_y: i32,
    },
    Key {
        key: String,
    },
    Selection {
        anchor_id: String,
        anchor_offset: u32,
        focus_id: String,
        focus_offset: u32,
    },
    Composition {
        node_id: String,
        text: String,
        selection_start: u32,
        selection_end: u32,
    },
    Coordinate {
        input: KernelBrowserInput,
    },
}
