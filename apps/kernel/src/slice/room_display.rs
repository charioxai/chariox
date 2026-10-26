//! MP-08/MP-10/MP-11: shared Room display negotiation. Pixels never grant input.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomDisplayOffer {
    pub codecs: Vec<String>,
    pub bitrate: u32,
    pub device_scale_factor: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_hint: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomDisplaySource {
    pub kind: String,
    pub surface_id: String,
    pub generation: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomDisplayBinding {
    pub generation: u64,
    pub subscription_id: String,
    pub codec: String,
    pub bitrate: u32,
    pub device_scale_factor: u32,
    pub source: RoomDisplaySource,
    #[serde(default)]
    pub resumed: bool,
}
