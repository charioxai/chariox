use serde::{Deserialize, Serialize};

use crate::runtime::browser_controller_process::{
    BrowserControllerProcessSnapshot, BrowserControllerReconciliation,
};
use crate::session::CanonicalViewport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserImportDomainStatus {
    Imported,
    NoCookies,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserImportDomainResult {
    pub(crate) domain: String,
    pub(crate) status: BrowserImportDomainStatus,
    pub(crate) cookie_count: u16,
}

pub(crate) const ROOM_COMPUTER_HOLD_MAX_DURATION_MS: u32 = 10_000;
pub(crate) const ROOM_COMPUTER_SCROLL_MAX_STEPS: u16 = 120;
pub(crate) const ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES: usize = 64 * 1024;
pub(crate) const ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES: usize = 128;
pub(crate) const ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT: u16 = 32;
pub(crate) const ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoomComputerPointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerSecretInput(String);

impl RoomComputerSecretInput {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerSecretInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerSecretInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer secret input]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerKeyboardInput(String);

impl RoomComputerKeyboardInput {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerKeyboardInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerKeyboardInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer keyboard input]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerClipboardText(String);

impl RoomComputerClipboardText {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerClipboardText {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerClipboardText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer clipboard text]")
    }
}

// MP-08 / MP-11: display-stack metadata only, bound before user approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoomComputerSecretTarget {
    pub(crate) focus_window: u64,
    pub(crate) active_window: u64,
    pub(crate) geometry: [i64; 4],
    pub(crate) window_geometry: [i64; 4],
}

impl RoomComputerSecretTarget {
    pub(crate) fn valid(&self) -> bool {
        self.focus_window > 1
            && self.active_window > 1
            && [self.geometry, self.window_geometry]
                .iter()
                .all(|geometry| {
                    geometry[2] > 0
                        && geometry[3] > 0
                        && geometry[2] <= 32768
                        && geometry[3] <= 32768
                })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomComputerInputAction {
    TargetAction {
        tree_revision: u64,
        target_id: String,
        action: String,
    },
    PointerMove {
        x: u32,
        y: u32,
    },
    PointerDrag {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        button: RoomComputerPointerButton,
    },
    PointerScroll {
        x: u32,
        y: u32,
        horizontal_steps: i16,
        vertical_steps: i16,
    },
    KeyboardText {
        input: RoomComputerKeyboardInput,
    },
    KeyboardKey {
        input: RoomComputerKeyboardInput,
        repeat: u16,
    },
    KeyboardHold {
        input: RoomComputerKeyboardInput,
        duration_ms: u32,
    },
    PointerHold {
        x: u32,
        y: u32,
        button: RoomComputerPointerButton,
        duration_ms: u32,
    },
    ClipboardWrite {
        text: RoomComputerClipboardText,
    },
    PointerClick {
        x: u32,
        y: u32,
        button: RoomComputerPointerButton,
        click_count: u8,
    },
    SecretText {
        input: RoomComputerSecretInput,
        expected_target: RoomComputerSecretTarget,
    },
}

/// Physical controller operations only. The home retains Room/tab authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BrowserLifecycleOperation {
    Tab {
        action: crate::runtime::browser_controller_tab::BrowserTabAction,
    },
    History {
        action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    },
    Navigate {
        url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl,
    },
    Dialog {
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoomBrowserImportBinding {
    pub(crate) request_id: String,
    pub(crate) user_id: String,
    pub(crate) room_id: String,
    pub(crate) environment_id: String,
}

/// Physical controller operations only. The home retains Room/tab authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerCommand {
    NoteObservation {
        target_id: String,
        document_id: String,
        quote: Option<crate::local::NoteTextQuote>,
    },
    Acquire,
    Reconcile {
        viewport: CanonicalViewport,
        /// The Room's browser bar; a worker older than the flag leaves
        /// windows as they are.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        browser_bar_visible: bool,
    },
    Artifact {
        request: crate::runtime::browser_artifact::BrowserArtifactRequest,
    },
    Snapshot {
        target_id: String,
        document_id: String,
    },
    Tab {
        execution_id: String,
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_tab::BrowserTabAction,
    },
    History {
        execution_id: String,
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    },
    Navigate {
        execution_id: String,
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
        execution_id: String,
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    },
    RecoverLifecycle {
        execution_id: String,
        target_id: String,
        document_id: String,
        operation: BrowserLifecycleOperation,
    },
    ConfigureDownloads {
        execution_id: String,
        target_id: String,
        document_id: String,
    },
    RecoverDownloadConfiguration {
        execution_id: String,
        target_id: String,
        document_id: String,
    },
    CancelDownload {
        cancellation: crate::runtime::browser_controller_file_transfer::BrowserDownloadCancellation,
    },
    Upload {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles,
    },
    RecoverUpload {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles,
    },
    Permission {
        execution_id: String,
        target_id: String,
        document_id: String,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    },
    RecoverPermission {
        execution_id: String,
        target_id: String,
        document_id: String,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    },
    AppView {
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
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
    ComputerSecretTarget,
    /// MP-08/MP-11: revoke retained values on an authenticated bound worker.
    ClearSecretObservation {
        #[serde(default)]
        disposition: SecretObservationDisposition,
    },
    ComputerClipboardRead {
        actor_id: String,
        runtime_generation: u64,
    },
    ImportCookies {
        binding: RoomBrowserImportBinding,
        browser_generation: u64,
        target_id: String,
        document_id: String,
        source_store_id: String,
        domains: Vec<String>,
        partition_sites: Vec<String>,
        overwrite: bool,
        payload: crate::runtime::browser_import_payload::BrowserImportPayload,
    },
    CancelCookieImport {
        request_id: String,
    },
    RecoverCookieImport {
        binding: RoomBrowserImportBinding,
        target_id: String,
    },
    Release,
}

// MP-08/MP-10/MP-11: authenticated worker lifecycle, never human clearance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecretObservationDisposition {
    #[default]
    Retire,
    ResetEnvironment,
    DeleteRoom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerResult {
    NoteObservation { observation: Option<crate::runtime::notes::observation::BrowserNoteObservation> },
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
    ComputerSecretTarget {
        target: RoomComputerSecretTarget,
    },
    ComputerInputApplied {
        action_id: String,
    },
    ComputerClipboard {
        content: RoomComputerClipboardText,
    },
    CookiesImported {
        results: Vec<BrowserImportDomainResult>,
    },
    SecretObservationCleared,
    CookieImportRecovered,
    CookieImportRolledBack,
    Artifact { capture: Option<crate::runtime::browser_artifact::BrowserArtifactCapture> },
    Snapshot {
        snapshot: Option<
            crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot,
        >,
    },
    Tab {
        result: Option<crate::runtime::browser_controller_tab::BrowserControllerTabResult>,
    },
    History {
        result: Option<
            crate::runtime::browser_controller_history::BrowserControllerHistoryResult,
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
    DownloadCancellation {
        result: Option<crate::runtime::browser_controller_file_transfer::BrowserControllerDownloadCancellationResult>,
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
    AppView {
        result: Option<serde_json::Value>,
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
