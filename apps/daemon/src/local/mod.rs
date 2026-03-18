mod api;
mod harness;
mod ipc;

pub use api::{
    AttachToSessionRequest, CancelActivePromptRequest, CaptureScreenshotCapabilityRequest,
    CompletePromptRequest, DetachFromSessionRequest, EditFileCapabilityRequest, EndSessionRequest,
    GetSessionStateRequest, InspectGitCapabilityRequest, LaunchProviderRunRequest,
    ListSessionsRequest, LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest,
    PumpTerminalOutputRequest, ReadDirectoryTreeCapabilityRequest, ReadFileCapabilityRequest,
    ResizeTerminalRequest, RunShellCapabilityRequest, StoreTransferredFileCapabilityRequest,
    SubmitPromptRequest, UpdateSessionConfigRequest,
};
pub use harness::{run_local_harness, LocalHarnessReport};
pub use ipc::{run_local_ipc_server, send_local_ipc_request, LocalIpcClient};
