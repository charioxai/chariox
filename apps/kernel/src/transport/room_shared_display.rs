//! MP-08/MP-10/MP-11: home-bound Room placement of the shared display protocol.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayCommand {
    Attach {},
    Capture {
        after_sequence: u64,
        reset: bool,
        congested: bool,
    },
    Suspend {},
    Resume {},
    // MP-08/MP-10/MP-11: 535/143 registration invokes525 and attaches locally.
    ResumeAttach {},
    Close {},
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SealedRoomDisplayFrame {
    pub sequence: u64,
    pub encrypted_event: chariox_relay::protocol::EncryptedRelayPayload,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayResult {
    Opened {
        binding: crate::slice::RoomDisplayBinding,
        peer_public_key: String,
    },
    Frame {
        frame: Option<SealedRoomDisplayFrame>,
    },
    Updated {
        binding: crate::slice::RoomDisplayBinding,
    },
    Settled {},
}

// MP-08/MP-11: kernel-resolved references only, never viewer rectangles/XIDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayTarget {
    Browser {
        tab_id: String,
        generation: u64,
    },
    Desktop {
        surface_id: String,
        generation: String,
    },
    Region {
        binding_id: String,
    },
}
