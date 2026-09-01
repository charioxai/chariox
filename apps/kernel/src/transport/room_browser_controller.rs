use serde::{Deserialize, Serialize};

use crate::runtime::browser_controller_process::{
    BrowserControllerProcessSnapshot, BrowserControllerReconciliation,
};
use crate::session::CanonicalViewport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoomComputerPointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomComputerInputAction {
    PointerClick {
        x: u32,
        y: u32,
        button: RoomComputerPointerButton,
        click_count: u8,
    },
}

/// Physical controller operations only. The home retains Room/tab authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerCommand {
    Acquire,
    Reconcile {
        viewport: CanonicalViewport,
    },
    Snapshot {
        target_id: String,
        document_id: String,
    },
    Navigate {
        target_id: String,
        document_id: String,
        url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl,
    },
    Wait {
        target_id: String,
        document_id: String,
        wait: crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait,
        timeout_ms: u64,
    },
    Dialog {
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    },
    ConfigureDownloads {
        target_id: String,
        document_id: String,
    },
    Upload {
        target_id: String,
        document_id: String,
        node_ref: String,
        files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles,
    },
    Permission {
        target_id: String,
        document_id: String,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    },
    PollEvents {
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    },
    Action {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    },
    RecoverAction {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    },
    CancelAction {
        execution_id: String,
    },
    ComputerInput {
        action_id: String,
        actor_id: String,
        runtime_generation: u64,
        viewport_revision: u64,
        desktop_pixel_width: u32,
        desktop_pixel_height: u32,
        action: RoomComputerInputAction,
    },
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerResult {
    RecoveryRequired {
        process: BrowserControllerProcessSnapshot,
    },
    ActionCancelled {
        controller_fenced: bool,
    },
    CancellationRequested {
        accepted: bool,
    },
    Action {
        result: Option<crate::runtime::browser_controller_action::BrowserControllerActionResult>,
    },
    ComputerInputApplied {
        action_id: String,
    },
    Snapshot {
        snapshot: Option<
            crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot,
        >,
    },
    Navigation {
        result: Option<
            crate::runtime::browser_controller_compatibility::BrowserControllerNavigationResult,
        >,
    },
    Wait {
        result: Option<
            crate::runtime::browser_controller_compatibility::BrowserControllerCompatibilityWaitResult,
        >,
    },
    Dialog {
        result: Option<crate::runtime::browser_controller_action::BrowserControllerDialogResult>,
    },
    Downloads {
        result: Option<
            crate::runtime::browser_controller_file_transfer::BrowserControllerDownloadsResult,
        >,
    },
    Upload {
        result:
            Option<crate::runtime::browser_controller_file_transfer::BrowserControllerUploadResult>,
    },
    Permission {
        result: Option<
            crate::runtime::browser_controller_permission::BrowserControllerPermissionResult,
        >,
    },
    Events {
        batch: Option<crate::runtime::browser_controller_event::BrowserControllerEventBatch>,
    },
    Process {
        snapshot: Option<BrowserControllerProcessSnapshot>,
    },
    Reconciled {
        reconciliation: Option<BrowserControllerReconciliation>,
    },
}
