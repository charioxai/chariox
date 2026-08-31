use serde::{Deserialize, Serialize};

use crate::runtime::browser_controller_process::{
    BrowserControllerProcessSnapshot, BrowserControllerReconciliation,
};
use crate::session::CanonicalViewport;

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
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerResult {
    ActionCancelled {
        controller_fenced: bool,
    },
    CancellationRequested {
        accepted: bool,
    },
    Action {
        result: Option<crate::runtime::browser_controller_action::BrowserControllerActionResult>,
    },
    Snapshot {
        snapshot: Option<
            crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot,
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
    Process {
        snapshot: Option<BrowserControllerProcessSnapshot>,
    },
    Reconciled {
        reconciliation: Option<BrowserControllerReconciliation>,
    },
}
