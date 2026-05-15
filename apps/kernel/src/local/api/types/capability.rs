use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunShellCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirectoryTreeCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: Option<PathBuf>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectGitCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureScreenshotCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreTransferredFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub source_path: PathBuf,
    pub display_name: Option<String>,
}
