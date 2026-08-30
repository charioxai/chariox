use super::*;

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
