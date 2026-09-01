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

pub type RoomEnvironmentPointerButton = crate::session::EnvironmentPointerButton;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoomEnvironmentHumanAction {
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
