use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindRoomEnvironmentSliceRequest {
    pub session_id: String,
    pub slice_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentSliceBinding {
    pub session_id: String,
    pub slice_id: String,
    pub owner_kernel_id: String,
    pub worker_kernel_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRoomEnvironmentSliceRequest {
    pub session_id: String,
}

/// Protocol 357: the accessibility outline of one Room Tab, for terminals to
/// present App views (and any page) to screen readers and keyboard users.
/// Room members may read it; it carries no input authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRoomEnvironmentTabAccessibilityRequest {
    pub session_id: String,
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomEnvironmentAccessibilityNode {
    pub element_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomEnvironmentTabAccessibility {
    pub session_id: String,
    pub tab_id: String,
    pub document_revision: u64,
    /// Nodes in document order; ignored nodes are omitted.
    pub nodes: Vec<RoomEnvironmentAccessibilityNode>,
    /// More nodes existed than the bound.
    pub truncated: bool,
}

/// Read the home-owned browser/profile identities for a Room-bound worker
/// environment. The slice id is part of the request so a stale worker or
/// unrelated slice cannot be projected as this Room's inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRoomEnvironmentResourceInventoryRequest {
    pub session_id: String,
    pub slice_id: String,
}

/// Opaque physical-resource identities observed by the bound worker after the
/// home kernel reconciles its browser controller. `profile_ids` are worker
/// observations (never Slice reservation IDs); the actual profile path never
/// crosses the API. The worker contract is scoped to this Room-bound worker
/// and must fail closed when its browser/profile inventory is not exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentResourceInventory {
    pub session_id: String,
    pub environment_id: String,
    pub slice_id: String,
    pub browser_ids: Vec<String>,
    pub profile_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRoomEnvironmentScreenshotRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoomEnvironmentScreenshotChunkRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub artifact_id: String,
    pub offset: u64,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentScreenshotArtifact {
    pub artifact_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentScreenshotChunk {
    pub artifact_id: String,
    pub offset: u64,
    pub data_base64: String,
    pub eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRoomEnvironmentStateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRoomEnvironmentEventsRequest {
    pub session_id: String,
    pub cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRoomEnvironmentActionHistoryRequest {
    pub session_id: String,
    #[serde(default)]
    pub before_sequence: Option<u64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentViewportRequest {
    pub css_width: u32,
    pub css_height: u32,
    pub device_scale_factor: u32,
    pub desktop_pixel_width: u32,
    pub desktop_pixel_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRoomEnvironmentRequest {
    pub session_id: String,
    pub viewport: RoomEnvironmentViewportRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRoomEnvironmentRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryRoomEnvironmentRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRoomEnvironmentViewportRequest {
    pub session_id: String,
    pub expected_revision: u64,
    pub viewport: RoomEnvironmentViewportRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEnvironmentPointerPositionRequest {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRoomEnvironmentPointerRequest {
    pub session_id: String,
    pub runtime_generation: u64,
    pub viewport_revision: u64,
    #[serde(default)]
    pub pointer: Option<RoomEnvironmentPointerPositionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestRoomEnvironmentInputTakeoverRequest {
    pub session_id: String,
    pub target: crate::session::InputTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseRoomEnvironmentInputRequest {
    pub session_id: String,
    pub target: crate::session::InputTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRoomEnvironmentActionRequest {
    pub session_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoomEnvironmentClipboardRequest {
    pub session_id: String,
    pub runtime_generation: u64,
}

pub type RoomEnvironmentPointerButton = crate::session::EnvironmentPointerButton;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomEnvironmentKeyboardInput(String);

impl RoomEnvironmentKeyboardInput {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for RoomEnvironmentKeyboardInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomEnvironmentKeyboardInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer keyboard input]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomEnvironmentClipboardText(String);

impl RoomEnvironmentClipboardText {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for RoomEnvironmentClipboardText {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomEnvironmentClipboardText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer clipboard text]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoomEnvironmentHumanAction {
    PointerMove {
        x: u32,
        y: u32,
    },
    PointerDrag {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        button: RoomEnvironmentPointerButton,
    },
    PointerScroll {
        x: u32,
        y: u32,
        horizontal_steps: i16,
        vertical_steps: i16,
    },
    KeyboardText {
        text: RoomEnvironmentKeyboardInput,
    },
    KeyboardKey {
        key: RoomEnvironmentKeyboardInput,
        repeat: u16,
    },
    ClipboardWrite {
        text: RoomEnvironmentClipboardText,
    },
    PointerClick {
        x: u32,
        y: u32,
        button: RoomEnvironmentPointerButton,
        click_count: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRoomEnvironmentActionRequest {
    pub session_id: String,
    pub runtime_generation: u64,
    pub viewport_revision: u64,
    pub idempotency_key: String,
    pub action: RoomEnvironmentHumanAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEnvironmentBrowserHistoryAction {
    Back,
    Forward,
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEnvironmentBrowserTabAction {
    Activate,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoomEnvironmentHumanBrowserAction {
    History {
        tab_id: String,
        action: RoomEnvironmentBrowserHistoryAction,
    },
    Tab {
        tab_id: String,
        action: RoomEnvironmentBrowserTabAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRoomEnvironmentBrowserActionRequest {
    pub session_id: String,
    pub runtime_generation: u64,
    pub idempotency_key: String,
    pub action: RoomEnvironmentHumanBrowserAction,
}
