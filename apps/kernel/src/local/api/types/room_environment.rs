use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRoomEnvironmentStateRequest {
    pub session_id: String,
}
