//! MD-2: sessionless host browser protocol (legacy 417; display 419).
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserCommand {
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
