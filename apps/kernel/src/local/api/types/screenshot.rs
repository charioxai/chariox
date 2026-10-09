//! Human screenshot selection, local protocol 443. Coordinates are relative
//! to the painted surface, excluding letterboxing, never the page viewport.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenshotSurface {
    Room {
        session_id: String,
        attachment_id: String,
        runtime_generation: u64,
        viewport_revision: u64,
    },
    KernelBrowser {
        tab_id: String,
        generation: u64,
    },
    UserAppView {
        view_id: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureVisibleRegionRequest {
    pub capture_id: String,
    pub surface: ScreenshotSurface,
    pub region: ScreenshotRegion,
}

/// Request-scoped event to the capturing client only. No session broadcast,
/// retained capture mailbox or new provider attachment authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibleRegionCapture {
    pub capture_id: String,
    pub surface: ScreenshotSurface,
    pub captured_at_ms: u64,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
    pub display_name: String,
    pub sha256: String,
    pub data_base64: String,
}
