use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

#[cfg(test)]
use super::browser_controller_action::BrowserDialogAction;
use super::browser_controller_action::{
    validate_browser_action_timeout, BrowserControllerActionResult, BrowserControllerDialogResult,
    BrowserLocatorAction,
};
use super::browser_controller_compatibility::{
    normalize_browser_navigation_url, BrowserCompatibilityWait,
    BrowserControllerCompatibilityWaitResult, BrowserControllerNavigationResult,
};
use super::browser_controller_event::{BrowserControllerEventBatch, MAX_BROWSER_EVENT_POLL_LIMIT};
use super::browser_controller_file_transfer::{
    BrowserControllerDownloadCancellationResult, BrowserControllerDownloadsResult,
    BrowserControllerUploadResult, BrowserDownloadCancellation, BrowserUploadFiles,
};
use super::browser_controller_history::BrowserControllerHistoryResult;
use super::browser_controller_permission::{
    BrowserControllerPermissionResult, BrowserPermissionName, BrowserPermissionSetting,
};
use super::browser_controller_snapshot::BrowserControllerStructuredSnapshot;
use super::browser_controller_tab::BrowserControllerTabResult;
use crate::session::CanonicalViewport;

mod app_view_bridge;
mod cancellation;
mod room_computer;
pub(crate) use cancellation::CancellationSignal as BrowserCancellation;
mod configuration_cancellation;
mod display_packets;
/// MP-08/MP-10/MP-11: native capture slot files (`<pool>/<index>`). Defined
/// here, outside the optional native worker, because crash cleanup runs in
/// every Unix build; the worker allocates exactly these.
pub(crate) const RASTER_SLOT_NAMES: [&str; 6] = ["0", "1", "2", "3", "4", "5"];
mod lifecycle_cancellation;
mod owned_process_group;
mod pending_action;
mod pending_mutation;
mod pending_responses;
mod reconciliation;
mod request_wire;
mod unlocked_request;
use self::pending_mutation::BrowserTabMutationLanes;
pub(crate) use configuration_cancellation::BrowserConfiguration;
#[cfg(test)]
mod action_concurrency_tests;
#[cfg(test)]
mod host_cancellation_tests;
#[cfg(test)]
mod import_cancellation_tests;
#[cfg(test)]
mod tab_mutation_concurrency_tests;
#[cfg(test)]
mod upload_cancellation_tests;

const DEFAULT_CONTROLLER_COMMAND_TIMEOUT_MS: u64 = 10_000;
const CONTROLLER_SCRIPT_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_SCRIPT";
const CONTROLLER_NODE_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_NODE";
const CONTROLLER_COMMAND_TIMEOUT_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_COMMAND_TIMEOUT_MS";
pub(crate) const CONTROLLER_RESTARTED_BEFORE_OPERATION: &str =
    "browser controller restarted before the operation; reconcile and retry with fresh references";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserControllerProcessState {
    Stopped,
    Starting,
    Ready,
    Unhealthy,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserControllerProcessHealth {
    pub(crate) state: BrowserControllerProcessState,
    pub(crate) process_id: Option<u32>,
    pub(crate) diagnostic_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserCookieImportOutcome {
    Applied(Vec<crate::transport::room_browser_controller::BrowserImportDomainResult>),
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerProcessSnapshot {
    pub(crate) state: BrowserControllerProcessState,
    pub(crate) process_id: Option<u32>,
    pub(crate) diagnostic_code: Option<String>,
    pub(crate) runtime_generation: u64,
    pub(crate) restart_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerReconciliation {
    pub(crate) process: BrowserControllerProcessSnapshot,
    pub(crate) browser: BrowserControllerBrowserSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerResourceInventory {
    pub(crate) browser_ids: Vec<String>,
    pub(crate) profile_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerBrowserSnapshot {
    pub(crate) browser_generation: u64,
    #[serde(default)]
    pub(crate) event_cursor: u64,
    pub(crate) tabs: Vec<BrowserControllerTabSnapshot>,
    pub(crate) focused_target_id: Option<String>,
    pub(crate) resource_inventory: BrowserControllerResourceInventory,
    viewport: BrowserControllerViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerTabSnapshot {
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
    pub(crate) title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserControllerViewport {
    css_width: u32,
    css_height: u32,
    device_scale_factor: u32,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
}

pub(crate) trait BrowserControllerProcessBackend {
    fn health(&mut self) -> Result<BrowserControllerProcessHealth, String>;
    fn start(&mut self) -> Result<BrowserControllerProcessHealth, String>;
    fn stop(&mut self) -> Result<(), String>;
    fn reconcile_browser(
        &mut self,
        _viewport: &CanonicalViewport,
        _browser_bar_visible: bool,
    ) -> Result<BrowserControllerBrowserSnapshot, String> {
        Err("browser controller backend does not support browser reconciliation".to_string())
    }

    #[cfg(test)]
    fn perform_browser_action(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _node_ref: &str,
        _action: &BrowserLocatorAction,
        _timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        Err("browser controller backend does not support locator actions".to_string())
    }

    #[cfg(test)]
    fn handle_browser_dialog(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        Err("browser controller backend does not support dialogs".to_string())
    }
    fn configure_browser_downloads(
        &mut self,
        _target_id: &str,
        _document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        Err("browser controller backend does not support downloads".to_string())
    }
    fn cancel_browser_download(
        &mut self,
        _cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        Err("browser controller backend does not support download cancellation".to_string())
    }
    fn set_browser_permission(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _permission: BrowserPermissionName,
        _setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        Err("browser controller backend does not support permissions".to_string())
    }
    fn browser_artifact(
        &mut self,
        _request: &super::browser_artifact::BrowserArtifactRequest,
    ) -> Result<super::browser_artifact::BrowserArtifactCapture, String> {
        Err("browser controller backend does not support Browser artifacts".into())
    }
    fn app_view(
        &mut self,
        _request: &crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<serde_json::Value, String> {
        Err("browser controller backend does not support App views".to_string())
    }
    fn poll_browser_events(
        &mut self,
        _browser_generation: u64,
        _cursor: u64,
        _limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        Err("browser controller backend does not support event polling".to_string())
    }
    fn import_browser_cookies(
        &mut self,
        _binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        _browser_generation: u64,
        _target_id: &str,
        _document_id: &str,
        _source_store_id: &str,
        _domains: &[String],
        _partition_sites: &[String],
        _overwrite: bool,
        _payload: &crate::runtime::browser_import_payload::BrowserImportPayload,
    ) -> Result<BrowserCookieImportOutcome, String> {
        Err("browser controller backend does not support cookie import".to_string())
    }
    fn recover_browser_cookie_import(
        &mut self,
        _binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        _target_id: &str,
    ) -> Result<(), String> {
        Err("browser controller backend does not support cookie recovery".to_string())
    }
}

pub(crate) struct BrowserControllerProcessStdioBackend {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    process: Option<BrowserControllerChild>,
    next_request_id: u64,
    host: bool,
    action_cancellation: Option<Arc<cancellation::CancellationSignal>>,
    protected_values: BTreeMap<String, Vec<zeroize::Zeroizing<String>>>,
}

impl BrowserControllerProcessStdioBackend {
    pub(crate) fn new(command: impl Into<PathBuf>, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            command: command.into(),
            args,
            timeout,
            process: None,
            next_request_id: 1,
            host: false,
            action_cancellation: None,
            protected_values: BTreeMap::new(),
        }
    }

    pub(crate) fn for_host(mut self) -> Self {
        self.host = true;
        self
    }

    fn from_script(script_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        let command = std::env::var_os(CONTROLLER_NODE_ENV)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "node".into());
        Self::new(
            PathBuf::from(command),
            vec![
                script_path.into().display().to_string(),
                "stdio".to_string(),
            ],
            timeout,
        )
    }

    fn spawn(&mut self) -> Result<(), String> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        if self.host {
            // MD-2: browser descendants receive OS display settings, never provider/control secrets.
            command.env_clear();
            for key in [
                "PATH",
                "HOME",
                "USER",
                "LOGNAME",
                "LANG",
                "LC_ALL",
                "DISPLAY",
                "WAYLAND_DISPLAY",
                "XAUTHORITY",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
                "TMPDIR",
                "CHARIOX_KERNEL_BROWSER_EXECUTABLE",
                "CHARIOX_KERNEL_BROWSER_HEADLESS",
                "CHARIOX_KERNEL_BROWSER_DISPLAY",
                "CHARIOX_KERNEL_BROWSER_MIRROR",
                "CHARIOX_BROWSER_DISPLAY_PYTHON",
                "CHARIOX_BROWSER_DISPLAY_TIMING",
                "CHARIOX_BROWSER_DISPLAY_GEOMETRY",
                "CHARIOX_BROWSER_DISPLAY_SOFTWARE",
                "LIBVA_DRIVER_NAME",
                "CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER",
                "CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER",
                "CHARIOX_BROWSER_DISPLAY_OPENH264",
                "CHARIOX_BROWSER_DISPLAY_LIBYUV",
                "CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
        }
        #[cfg(all(feature = "native-display", target_os = "linux"))]
        if self.host {
            // Production uses this exact kernel ELF. Test harnesses provide the
            // separately built production ELF because libtest owns their entry.
            let executable = if cfg!(test) {
                std::env::var_os("CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER").map(PathBuf::from)
            } else {
                std::env::current_exe().ok()
            };
            if let Some(executable) = executable {
                command.env("CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER", executable);
            }
        }
        // MP-08/MP-10/MP-11: a kernel-created descriptor root, never an ambient path.
        let display_packets = if self.host && cfg!(unix) {
            Some(Arc::new(display_packets::DisplayPackets::create()?))
        } else {
            None
        };
        if let Some(packets) = &display_packets {
            command.env("CHARIOX_BROWSER_DISPLAY_PACKET_ROOT", packets.root());
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to spawn browser controller `{}`: {error}",
                self.command.display()
            )
        })?;
        let mut owned_group = owned_process_group::OwnedProcessGroup::new(child.id());
        if self.host {
            if let Err(error) = owned_group.start_tracking() {
                kill_owned_child(&mut child, &mut owned_group);
                return Err(format!("failed to track browser descendants: {error}"));
            }
        }
        let stdin = child.stdin.take().ok_or_else(|| {
            kill_owned_child(&mut child, &mut owned_group);
            "browser controller did not expose stdin".to_string()
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            kill_owned_child(&mut child, &mut owned_group);
            "browser controller did not expose stdout".to_string()
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            kill_owned_child(&mut child, &mut owned_group);
            "browser controller did not expose stderr".to_string()
        })?;
        let (responses_tx, responses) = mpsc::channel();
        let pending_responses = pending_responses::PendingResponses::default();
        let reader_pending_responses = pending_responses.clone();
        let reader_ownership = self.host.then(|| owned_group.witness());
        if let Err(error) = std::thread::Builder::new()
            .name("chariox-browser-controller-reader".to_string())
            .spawn(move || {
                read_controller_responses(
                    stdout,
                    responses_tx,
                    reader_pending_responses,
                    reader_ownership,
                    display_packets,
                )
            })
        {
            kill_owned_child(&mut child, &mut owned_group);
            return Err(format!(
                "failed to start browser controller response reader: {error}"
            ));
        }
        let _ = std::thread::Builder::new()
            .name("chariox-browser-controller-stderr".to_string())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    if line.is_err() {
                        break;
                    }
                }
            });
        self.process = Some(BrowserControllerChild {
            owned_group,
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            responses,
            pending_responses,
            host_policy: None,
        });
        Ok(())
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<BrowserControllerRpcResponse, String> {
        // Locator actions have their own bounded actionability deadline inside
        // the controller. The stdio deadline starts outside that operation, so
        // add the declared action budget instead of timing the transport out
        // while the controller is still performing or cleaning up the action.
        let timeout = if method == "browser.action" {
            self.timeout.saturating_add(Duration::from_millis(
                params
                    .get("timeout_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            ))
        } else if method == "browser.reconcile" {
            // Physical display verification includes bounded streamer readback
            // and rollback. Do not kill its controller during that cleanup.
            self.timeout.max(Duration::from_secs(45))
        } else {
            self.timeout
        };
        self.request_serializable(method, &params, timeout)
    }

    // MD-2: bounded host adapter RPC; public callers never choose the method.
    pub(crate) fn host_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let lifecycle = self.host
            && method == "host.browser"
            && matches!(
                params.get("op").and_then(serde_json::Value::as_str),
                Some("start" | "open" | "state" | "display_subscribe")
            );
        let result = self.request(method, params)?.into_result(method);
        if lifecycle {
            if let Some(process) = self.process.as_mut() {
                process.owned_group.refresh();
            }
        }
        result
    }

    pub(crate) fn host_request_classified(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, crate::error::HostFailure> {
        let response = self
            .request(method, params)
            .map_err(crate::error::HostFailure::Other)?;
        if self.host && !response.ok {
            if let Some(reason) = response
                .error
                .as_ref()
                .and_then(|error| crate::error::UserDomainRefusalReason::from_code(&error.code))
            {
                return Err(crate::error::HostFailure::Refused(reason));
            }
        }
        response
            .into_result(method)
            .map_err(crate::error::HostFailure::Other)
    }

    // MP-08/MP-10/MP-11: per-credit liveness never joins the controller's
    // barrier lane. Startup still validates health; requests validate replies.
    pub(crate) fn host_is_live(&mut self) -> Result<bool, String> {
        self.take_exited_process()?;
        Ok(self.process.is_some())
    }

    // Cache only successfully applied policy on this exact supervised child.
    // A changed policy remains an RPC barrier; respawn cannot inherit the cache.
    pub(crate) fn protect_host(
        &mut self,
        policy: serde_json::Value,
    ) -> Result<(), crate::error::HostFailure> {
        self.take_exited_process()
            .map_err(crate::error::HostFailure::Other)?;
        if self
            .process
            .as_ref()
            .is_some_and(|p| p.host_policy.as_ref() == Some(&policy))
        {
            return Ok(());
        }
        self.host_request_classified("host.protect", policy.clone())?;
        if let Some(process) = self.process.as_mut() {
            process.host_policy = Some(policy);
        }
        Ok(())
    }

    // MD-DISPLAY-04: host RPCs validate their own outcome. The host health RPC
    // reports only this same supervisor PID, so don't repeat it per frame.
    // Keep exact owned-child exit detection and cold/recovery health admission.
    pub(crate) fn ensure_host_started(&mut self) -> Result<bool, String> {
        if !self.host {
            return Err("host controller required".into());
        }
        self.take_exited_process()?;
        if self.process.is_none() {
            self.start()?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn host_request_cancellable(
        &mut self,
        method: &str,
        params: serde_json::Value,
        signal: Option<Arc<BrowserCancellation>>,
    ) -> Result<serde_json::Value, crate::error::HostFailure> {
        let previous = std::mem::replace(&mut self.action_cancellation, signal);
        let result = self.host_request_classified(method, params);
        self.action_cancellation = previous;
        result
    }

    fn request_serializable<P: Serialize>(
        &mut self,
        method: &str,
        params: &P,
        timeout: Duration,
    ) -> Result<BrowserControllerRpcResponse, String> {
        let cancellation = matches!(
            method,
            "host.computer"
                | "host.computer.reset"
                | "host.browser"
                | "host.secret"
                | "browser.action"
                | "browser.upload"
                | "browser.downloads.configure"
                | "browser.permission"
                | "browser.tab"
                | "browser.navigate"
                | "browser.history"
                | "browser.dialog"
                | "browser.cookies.import"
        )
        .then(|| self.action_cancellation.clone())
        .flatten();
        if cancellation
            .as_ref()
            .is_some_and(|signal| signal.requested())
        {
            cancellation.as_ref().unwrap().confirm_stop();
            return Err("browser action cancelled before dispatch".into());
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| "browser controller is not running".to_string())?;
        let ownership_at = Instant::now();
        if !self.host {
            process.owned_group.refresh();
        }
        crate::transport::kernel_browser_display::timing("process_identity_refresh", ownership_at);
        let mut stdin = process
            .stdin
            .lock()
            .map_err(|_| "controller stdin lock poisoned")?;
        request_wire::write_line(
            &mut *stdin,
            &BrowserControllerRpcRequest {
                id: request_id,
                method,
                params,
                protected_values: self
                    .protected_values
                    .values()
                    .flatten()
                    .map(|value| value.as_str())
                    .collect(),
            },
        )
        .map_err(|error| format!("failed to encode browser controller request: {error}"))?;
        drop(stdin);
        let started = Instant::now();
        let mut cancellation_sent = false;
        let mut cancellation_request_id = None;
        let mut cancellation_acknowledged = false;
        let mut terminal_response = None;
        loop {
            if !cancellation_sent
                && cancellation
                    .as_ref()
                    .is_some_and(|signal| signal.requested())
            {
                let cancel_id = self.next_request_id;
                self.next_request_id = self.next_request_id.saturating_add(1);
                let mut stdin = process
                    .stdin
                    .lock()
                    .map_err(|_| "controller stdin lock poisoned")?;
                request_wire::write_line(
                    &mut *stdin,
                    &serde_json::json!({
                        "id":cancel_id,"method":"browser.cancel","params":{"request_id":request_id}
                    }),
                )
                .map_err(|error| error.to_string())?;
                cancellation_sent = true;
                cancellation_request_id = Some(cancel_id);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                if let Some(signal) = cancellation.as_ref().filter(|signal| signal.requested()) {
                    // A timeout is not proof that physical input stopped. Kill
                    // and reap the only process capable of sending more input
                    // before confirming cancellation to the home kernel.
                    kill_owned_child(
                        &mut process
                            .child
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()),
                        &mut process.owned_group,
                    );
                    signal.confirm_fence();
                    return Ok(BrowserControllerRpcResponse {
                        id: Some(request_id),
                        ok: false,
                        result: None,
                        error: Some(BrowserControllerRpcError {
                            code: "browser_action_cancelled".to_string(),
                            message: "browser controller was fenced after cancellation timed out"
                                .to_string(),
                        }),
                    });
                }
                return Err(format!(
                    "browser controller `{method}` timed out after {}ms",
                    timeout.as_millis()
                ));
            }
            let poll = if cancellation.is_some() {
                remaining.min(Duration::from_millis(20))
            } else {
                remaining
            };
            let response = match process.responses.recv_timeout(poll) {
                Ok(response) => response?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!("browser controller exited during `{method}`"))
                }
            };
            let ownership_at = Instant::now();
            if !self.host {
                process.owned_group.refresh();
            }
            crate::transport::kernel_browser_display::timing(
                "process_identity_refresh",
                ownership_at,
            );
            if response.id == cancellation_request_id {
                cancellation_acknowledged = true;
                let accepted = response.ok
                    && response
                        .result
                        .as_ref()
                        .and_then(|value| value.get("accepted"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(true);
                if let Some(signal) = &cancellation {
                    if accepted {
                        signal.confirm_stop();
                    } else {
                        signal.reject_after_stop();
                    }
                }
                if let Some(response) = terminal_response.take() {
                    return Ok(response);
                }
                continue;
            }
            if response.id == Some(request_id) {
                if !response.ok
                    && response
                        .error
                        .as_ref()
                        .is_some_and(|error| error.code == "browser_action_cancelled")
                {
                    if let Some(signal) = &cancellation {
                        signal.confirm_stop();
                    }
                }
                if cancellation_sent {
                    if cancellation_acknowledged {
                        return Ok(response);
                    }
                    terminal_response = Some(response);
                    continue;
                }
                return Ok(response);
            }
        }
    }

    fn health_request(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        let process_id = self
            .process
            .as_ref()
            .map(|process| {
                process
                    .child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .id()
            })
            .ok_or_else(|| "browser controller is not running".to_string())?;
        let response = self.request("health", serde_json::json!({}))?;
        let health = response.into_result::<BrowserControllerCommandHealth>("health")?;
        let health = health.into_health("health")?;
        if health.process_id != Some(process_id) {
            return Err(format!(
                "browser controller health reported process {:?}, expected {process_id}",
                health.process_id
            ));
        }
        Ok(health)
    }

    fn begin_snapshot_read(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<pending_responses::PendingResponse<BrowserControllerRpcResponse>, String> {
        self.begin_observation_request(
            "browser.snapshot",
            serde_json::json!({
                "target_id": target_id, "document_id": document_id,
            }),
        )
    }

    fn take_exited_process(&mut self) -> Result<Option<u32>, String> {
        let Some(process) = self.process.as_mut() else {
            return Ok(None);
        };
        let mut child = process
            .child
            .lock()
            .map_err(|_| "controller child lock poisoned")?;
        let process_id = child.id();
        let status = child
            .try_wait()
            .map_err(|error| format!("failed to inspect browser controller: {error}"))?;
        drop(child);
        if status.is_some() {
            // MD-2: reparented descendants must match identities recorded while owned.
            if self.host {
                process.owned_group.signal();
            }
            self.process.take();
            return Ok(Some(process_id));
        }
        Ok(None)
    }
}

#[derive(Serialize)]
struct BrowserControllerRpcRequest<'a, P> {
    id: u64,
    method: &'a str,
    params: &'a P,
    protected_values: Vec<&'a str>,
}

/// How a controller error names its code in the error text, which reaches
/// the home kernel unchanged through the relay; a caller that classifies
/// controller errors (a Room start's retry) matches this marker.
pub(crate) fn controller_error_marker(code: &str) -> String {
    format!("failed with {code}:")
}

impl BrowserControllerRpcResponse {
    pub(crate) fn into_host_result(
        self,
        method: &str,
    ) -> Result<serde_json::Value, crate::error::HostFailure> {
        if !self.ok {
            if let Some(reason) = self
                .error
                .as_ref()
                .and_then(|error| crate::error::UserDomainRefusalReason::from_code(&error.code))
            {
                return Err(crate::error::HostFailure::Refused(reason));
            }
        }
        self.into_result(method)
            .map_err(crate::error::HostFailure::Other)
    }
    pub(crate) fn into_result<T: DeserializeOwned>(self, method: &str) -> Result<T, String> {
        if !self.ok {
            let error = self.error.unwrap_or(BrowserControllerRpcError {
                code: "controller_error".to_string(),
                message: "browser controller returned an unspecified error".to_string(),
            });
            return Err(format!(
                "browser controller `{method}` {} {}",
                controller_error_marker(&error.code),
                error.message
            ));
        }
        let result = self
            .result
            .ok_or_else(|| format!("browser controller `{method}` omitted its result"))?;
        serde_json::from_value(result).map_err(|error| {
            format!("browser controller `{method}` returned invalid result: {error}")
        })
    }
}

impl BrowserControllerCommandHealth {
    fn into_health(self, method: &str) -> Result<BrowserControllerProcessHealth, String> {
        let state = match self.state.as_str() {
            "stopped" => BrowserControllerProcessState::Stopped,
            "starting" => BrowserControllerProcessState::Starting,
            "ready" => BrowserControllerProcessState::Ready,
            "unhealthy" => BrowserControllerProcessState::Unhealthy,
            "failed" => BrowserControllerProcessState::Failed,
            state => {
                return Err(format!(
                    "browser controller `{method}` returned unknown state `{state}`"
                ));
            }
        };
        Ok(BrowserControllerProcessHealth {
            state,
            process_id: self.process_id,
            diagnostic_code: self.diagnostic_code,
        })
    }
}

impl BrowserControllerProcessBackend for BrowserControllerProcessStdioBackend {
    fn health(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        if let Some(process_id) = self.take_exited_process()? {
            return Ok(BrowserControllerProcessHealth {
                state: BrowserControllerProcessState::Unhealthy,
                process_id: Some(process_id),
                diagnostic_code: Some("process_exited".to_string()),
            });
        }
        if self.process.is_none() {
            return Ok(BrowserControllerProcessHealth {
                state: BrowserControllerProcessState::Stopped,
                process_id: None,
                diagnostic_code: None,
            });
        }
        self.health_request()
    }

    fn start(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        if self.process.is_some() {
            self.stop()?;
        }
        self.spawn()?;
        match self.health_request() {
            Ok(health) => Ok(health),
            Err(error) => {
                if let Some(mut process) = self.process.take() {
                    kill_owned_child(
                        &mut process
                            .child
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()),
                        &mut process.owned_group,
                    );
                }
                Err(error)
            }
        }
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.process.is_none() {
            return Ok(());
        }
        let shutdown_requested = self.request("shutdown", serde_json::json!({})).is_ok();
        if let Some(mut process) = self.process.take() {
            if shutdown_requested {
                terminate_child(
                    &mut process
                        .child
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()),
                    &mut process.owned_group,
                    self.timeout,
                );
            } else {
                kill_owned_child(
                    &mut process
                        .child
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()),
                    &mut process.owned_group,
                );
            }
        }
        Ok(())
    }

    fn reconcile_browser(
        &mut self,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<BrowserControllerBrowserSnapshot, String> {
        let response = self.request(
            "browser.reconcile",
            browser_reconcile_params(viewport, browser_bar_visible),
        )?;
        let snapshot =
            response.into_result::<BrowserControllerBrowserSnapshot>("browser.reconcile")?;
        snapshot.validate(viewport)?;
        Ok(snapshot)
    }

    #[cfg(test)]
    fn perform_browser_action(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        action.validate()?;
        validate_browser_action_timeout(timeout_ms)?;
        let response = self.request(
            "browser.action",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "node_ref": node_ref,
                "action": action.controller_value(),
                "timeout_ms": timeout_ms,
            }),
        )?;
        let result = response.into_result::<BrowserControllerActionResult>("browser.action")?;
        result.validate(target_id, document_id, action.kind())?;
        Ok(result)
    }

    #[cfg(test)]
    fn handle_browser_dialog(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        action.validate()?;
        let response = self.request(
            "browser.dialog",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "action": action.kind(),
                "prompt_text": action.prompt_text(),
            }),
        )?;
        let result = response.into_result::<BrowserControllerDialogResult>("browser.dialog")?;
        result.validate(target_id, document_id, action)?;
        Ok(result)
    }

    fn configure_browser_downloads(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        let response = self.request(
            "browser.downloads.configure",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
            }),
        )?;
        let result = response
            .into_result::<BrowserControllerDownloadsResult>("browser.downloads.configure")?;
        result.validate(target_id, document_id)?;
        Ok(result)
    }

    fn cancel_browser_download(
        &mut self,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        let response = self.request(
            "browser.downloads.cancel",
            serde_json::to_value(cancellation).map_err(|error| error.to_string())?,
        )?;
        let result = response.into_result::<BrowserControllerDownloadCancellationResult>(
            "browser.downloads.cancel",
        )?;
        result.validate(cancellation)?;
        Ok(result)
    }

    fn set_browser_permission(
        &mut self,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        let response = self.request(
            "browser.permission",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "permission": permission.as_str(),
                "setting": setting.as_str(),
            }),
        )?;
        let result =
            response.into_result::<BrowserControllerPermissionResult>("browser.permission")?;
        result.validate(target_id, document_id, permission, setting)?;
        Ok(result)
    }

    fn browser_artifact(
        &mut self,
        request: &super::browser_artifact::BrowserArtifactRequest,
    ) -> Result<super::browser_artifact::BrowserArtifactCapture, String> {
        let result = self
            .request_serializable("browser.artifact", request, self.timeout)?
            .into_result::<super::browser_artifact::BrowserArtifactCapture>("browser.artifact")?;
        result.validate(request)?;
        Ok(result)
    }
    fn app_view(
        &mut self,
        request: &crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<serde_json::Value, String> {
        let method = request.method();
        let timeout = self.timeout;
        self.request_serializable(method, &request.params(), timeout)?
            .into_result(method)
    }

    fn poll_browser_events(
        &mut self,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        if browser_generation == 0 {
            return Err("browser event generation must be positive".to_string());
        }
        if limit == 0 || limit > MAX_BROWSER_EVENT_POLL_LIMIT {
            return Err(format!(
                "browser event limit must be between 1 and {MAX_BROWSER_EVENT_POLL_LIMIT}"
            ));
        }
        let response = self.request(
            "browser.events.poll",
            serde_json::json!({
                "browser_generation": browser_generation,
                "cursor": cursor,
                "limit": limit,
            }),
        )?;
        let result = response.into_result::<BrowserControllerEventBatch>("browser.events.poll")?;
        result.validate(browser_generation, cursor, limit)?;
        Ok(result)
    }

    fn import_browser_cookies(
        &mut self,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        browser_generation: u64,
        target_id: &str,
        document_id: &str,
        source_store_id: &str,
        domains: &[String],
        partition_sites: &[String],
        overwrite: bool,
        payload: &crate::runtime::browser_import_payload::BrowserImportPayload,
    ) -> Result<BrowserCookieImportOutcome, String> {
        #[derive(Serialize)]
        struct Params<'a> {
            binding: &'a crate::transport::room_browser_controller::RoomBrowserImportBinding,
            browser_generation: u64,
            target_id: &'a str,
            document_id: &'a str,
            source_store_id: &'a str,
            domains: &'a [String],
            partition_sites: &'a [String],
            overwrite: bool,
            payload_json: &'a str,
        }
        let params = Params {
            binding,
            browser_generation,
            target_id,
            document_id,
            source_store_id,
            domains,
            partition_sites,
            overwrite,
            payload_json: payload.as_str(),
        };
        let response =
            self.request_serializable("browser.cookies.import", &params, self.timeout)?;
        let result = response.into_result::<BrowserCookieImportResult>("browser.cookies.import")?;
        match result.status.as_str() {
            "applied" if validate_domain_results(&result.results, domains) => {
                Ok(BrowserCookieImportOutcome::Applied(result.results))
            }
            "rolled_back" if result.results.is_empty() => {
                Ok(BrowserCookieImportOutcome::RolledBack)
            }
            _ => Err("browser cookie import returned an invalid outcome".to_string()),
        }
    }

    fn recover_browser_cookie_import(
        &mut self,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        target_id: &str,
    ) -> Result<(), String> {
        let response = self.request(
            "browser.cookies.recover",
            serde_json::json!({
                "binding": binding, "target_id": target_id,
            }),
        )?;
        let result =
            response.into_result::<BrowserCookieRecoveryResult>("browser.cookies.recover")?;
        if result.status == "verified" {
            Ok(())
        } else {
            Err("browser cookie recovery returned an invalid outcome".into())
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCookieImportResult {
    status: String,
    results: Vec<crate::transport::room_browser_controller::BrowserImportDomainResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCookieRecoveryResult {
    status: String,
}

fn validate_domain_results(
    results: &[crate::transport::room_browser_controller::BrowserImportDomainResult],
    domains: &[String],
) -> bool {
    results.len() == domains.len()
        && results.iter().zip(domains).all(|(result, domain)| {
            result.domain == *domain && result.cookie_count <= 512 && match result.status {
                crate::transport::room_browser_controller::BrowserImportDomainStatus::Imported => {
                    result.cookie_count > 0
                }
                crate::transport::room_browser_controller::BrowserImportDomainStatus::NoCookies => {
                    result.cookie_count == 0
                }
            }
        })
        && results
            .iter()
            .map(|result| usize::from(result.cookie_count))
            .sum::<usize>()
            <= 512
}

impl Drop for BrowserControllerProcessStdioBackend {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct BrowserControllerChild {
    owned_group: owned_process_group::OwnedProcessGroup,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    responses: mpsc::Receiver<Result<BrowserControllerRpcResponse, String>>,
    pending_responses: pending_responses::PendingResponses<BrowserControllerRpcResponse>,
    host_policy: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct BrowserControllerCommandHealth {
    state: String,
    process_id: Option<u32>,
    diagnostic_code: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct BrowserControllerRpcResponse {
    id: Option<u64>,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<BrowserControllerRpcError>,
}

impl BrowserControllerBrowserSnapshot {
    fn validate(&self, expected_viewport: &CanonicalViewport) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser controller returned zero browser generation".to_string());
        }
        self.resource_inventory.validate()?;
        if self.viewport.css_width != expected_viewport.css_width
            || self.viewport.css_height != expected_viewport.css_height
            || self.viewport.device_scale_factor != expected_viewport.device_scale_factor
            || self.viewport.desktop_pixel_width != expected_viewport.desktop_pixel_width
            || self.viewport.desktop_pixel_height != expected_viewport.desktop_pixel_height
        {
            return Err("browser controller did not apply the canonical viewport".to_string());
        }
        let mut target_ids = BTreeSet::new();
        for tab in &self.tabs {
            if tab.target_id.is_empty() || tab.document_id.is_empty() {
                return Err(
                    "browser controller returned an empty target or document identity".to_string(),
                );
            }
            if !target_ids.insert(tab.target_id.as_str()) {
                return Err(format!(
                    "browser controller returned duplicate target `{}`",
                    tab.target_id
                ));
            }
        }
        if let Some(focused_target_id) = &self.focused_target_id {
            if !target_ids.contains(focused_target_id.as_str()) {
                return Err(format!(
                    "browser controller focused unknown target `{focused_target_id}`"
                ));
            }
        }
        Ok(())
    }
}

impl BrowserControllerResourceInventory {
    fn validate(&self) -> Result<(), String> {
        validate_resource_identities(&self.browser_ids, "browser")?;
        validate_resource_identities(&self.profile_ids, "profile")?;
        Ok(())
    }
}

fn validate_resource_identities(identities: &[String], kind: &str) -> Result<(), String> {
    let mut unique = BTreeSet::new();
    for identity in identities {
        if identity.trim().is_empty() {
            return Err(format!(
                "browser controller returned an empty {kind} identity"
            ));
        }
        if !unique.insert(identity) {
            return Err(format!(
                "browser controller returned duplicate {kind} identity `{identity}`"
            ));
        }
    }
    if identities.len() != 1 {
        return Err(format!(
            "browser controller must observe exactly one {kind} identity, got {}",
            identities.len()
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct BrowserControllerRpcError {
    code: String,
    message: String,
}

fn read_controller_responses(
    stdout: ChildStdout,
    responses: mpsc::Sender<Result<BrowserControllerRpcResponse, String>>,
    pending_responses: pending_responses::PendingResponses<BrowserControllerRpcResponse>,
    mut ownership: Option<owned_process_group::OwnedProcessGroup>,
    display_packets: Option<Arc<display_packets::DisplayPackets>>,
) {
    for line in BufReader::new(stdout).lines() {
        let response = line
            .map_err(|error| format!("failed to read browser controller response: {error}"))
            .and_then(|line| {
                serde_json::from_str::<BrowserControllerRpcResponse>(&line)
                    .map_err(|error| format!("browser controller returned invalid JSON: {error}"))
                    .and_then(|mut response| {
                        if let Some(result) = &mut response.result {
                            if let Some(packets) = &display_packets {
                                packets.hydrate(result)?;
                            } else if result.pointer("/display_frame/native_packet").is_some() {
                                return Err("MP-11: native packet without kernel ownership".into());
                            }
                        }
                        Ok(response)
                    })
            });
        let response = match response {
            Ok(response) => {
                // Cold capture/encoder replacement returns an independent frame.
                // Record lifecycle identities before publishing it; deltas never
                // scan here. The observer covers launches during idle/failed RPCs.
                if response.result.as_ref().is_some_and(|v| {
                    v.pointer("/display_frame/key")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                        || v.pointer("/display_frame/sequence")
                            .and_then(serde_json::Value::as_u64)
                            == Some(1)
                }) {
                    if let Some(group) = ownership.as_mut() {
                        group.refresh();
                    }
                }
                if let Some(id) = response.id {
                    match pending_responses.route(id, response) {
                        Some(response) => Ok(response),
                        None => continue,
                    }
                } else {
                    Ok(response)
                }
            }
            Err(error) => {
                pending_responses.fail_all(&error);
                Err(error)
            }
        };
        if responses.send(response).is_err() {
            break;
        }
    }
    pending_responses.fail_all_on_exit();
}

fn terminate_child(
    child: &mut Child,
    group: &mut owned_process_group::OwnedProcessGroup,
    timeout: Duration,
) {
    let _ = child.wait_timeout(timeout);
    // MD-4: even a successful shutdown can leave recorded browser descendants.
    kill_owned_child(child, group);
}

fn kill_child(child: &mut Child) {
    let mut group = owned_process_group::OwnedProcessGroup::new(child.id());
    kill_owned_child(child, &mut group);
}

fn kill_owned_child(child: &mut Child, group: &mut owned_process_group::OwnedProcessGroup) {
    if child.id() <= 1 || child.id() > i32::MAX as u32 {
        return;
    }
    // MD-4: preserve start identities recorded before the controller exited.
    group.signal();
    let _ = child.kill();
    let _ = child.wait();
}

/// `browser.reconcile` parameters for both the barrier and observation paths.
fn browser_reconcile_params(
    viewport: &CanonicalViewport,
    browser_bar_visible: bool,
) -> serde_json::Value {
    serde_json::json!({
        "viewport": {
            "css_width": viewport.css_width,
            "css_height": viewport.css_height,
            "device_scale_factor": viewport.device_scale_factor,
            "desktop_pixel_width": viewport.desktop_pixel_width,
            "desktop_pixel_height": viewport.desktop_pixel_height,
        },
        "browser_bar_visible": browser_bar_visible,
        // App pages lay out left of the trusted conversation panel.
        "app_panel_css_width": viewport.app_panel_css_width(),
    })
}

pub(crate) struct BrowserControllerProcessSupervisor<B> {
    backend: B,
    snapshot: BrowserControllerProcessSnapshot,
    recovery_pending: bool,
    reconciled_viewport: Option<CanonicalViewport>,
    /// Window mode of the last barrier reconcile. Toggling the browser bar
    /// changes every window, so it cannot take the observation path.
    reconciled_browser_bar_visible: bool,
}

type StdioOwnership = BrowserControllerProcessOwnership<BrowserControllerProcessStdioBackend>;

pub(crate) struct BrowserControllerProcessOwnership<B> {
    supervisor: BrowserControllerProcessSupervisor<B>,
    // One backend addresses one physical browser/profile. Stopping the
    // controller does not reset that profile or make it safe for another Room.
    owner_session_id: Option<String>,
    leased: bool,
}

impl<B: BrowserControllerProcessBackend> BrowserControllerProcessOwnership<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            supervisor: BrowserControllerProcessSupervisor::new(backend),
            owner_session_id: None,
            leased: false,
        }
    }

    pub(crate) fn acquire(
        &mut self,
        session_id: &str,
    ) -> Result<BrowserControllerProcessSnapshot, String> {
        if session_id.trim().is_empty() {
            return Err("browser controller requires a Room identity".to_string());
        }
        if let Some(owner) = &self.owner_session_id {
            if owner != session_id {
                return Err("browser controller is bound to another Room".to_string());
            }
        } else {
            // Reserve before startup: a failed health check can still leave a
            // browser/profile behind, so it must not permit reassignment.
            self.owner_session_id = Some(session_id.to_string());
        }
        let snapshot = self.supervisor.ensure_started()?.clone();
        self.leased = true;
        Ok(snapshot)
    }

    pub(crate) fn release(
        &mut self,
        session_id: &str,
    ) -> Result<BrowserControllerProcessSnapshot, String> {
        if self.owner_session_id.as_deref() == Some(session_id) {
            self.supervisor.stop()?;
            self.leased = false;
        }
        Ok(self.supervisor.snapshot().clone())
    }

    pub(crate) fn reconcile_browser(
        &mut self,
        session_id: &str,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<BrowserControllerReconciliation, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .reconcile_browser(viewport, browser_bar_visible)
    }

    #[cfg(test)]
    pub(crate) fn perform_browser_action(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .perform_browser_action(target_id, document_id, node_ref, action, timeout_ms)
    }

    #[cfg(test)]
    pub(crate) fn handle_browser_dialog(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .handle_browser_dialog(target_id, document_id, action)
    }

    pub(crate) fn configure_browser_downloads(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .configure_browser_downloads(target_id, document_id)
    }

    pub(crate) fn cancel_browser_download(
        &mut self,
        session_id: &str,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        self.require_lease(session_id)?;
        self.supervisor.cancel_browser_download(cancellation)
    }

    pub(crate) fn set_browser_permission(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .set_browser_permission(target_id, document_id, permission, setting)
    }

    fn require_app_view_lease(&self, session_id: &str) -> Result<(), String> {
        self.require_lease(session_id).map_err(|_| {
            match self.owner_session_id.as_deref() {
                Some(owner) if owner != session_id => format!(
                    "This browser Environment belongs to Room {owner}, not Room {session_id}. Attach to that Room and use /app open <installation-id>, then /room view; or bind a separate Environment to this Room with /room bind <slice>."
                ),
                _ => "This Room's browser Environment is not started. Use /room start, then retry /app open <installation-id>.".to_owned(),
            }
        })
    }

    pub(crate) fn browser_artifact(
        &mut self,
        session_id: &str,
        request: &super::browser_artifact::BrowserArtifactRequest,
    ) -> Result<super::browser_artifact::BrowserArtifactCapture, String> {
        self.require_lease(session_id)?;
        self.supervisor.browser_artifact(request)
    }
    pub(crate) fn app_view(
        &mut self,
        session_id: &str,
        request: &crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<serde_json::Value, String> {
        self.require_app_view_lease(session_id)?;
        self.supervisor.app_view(request)
    }

    pub(crate) fn poll_browser_events(
        &mut self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .poll_browser_events(browser_generation, cursor, limit)
    }

    pub(crate) fn import_browser_cookies(
        &mut self,
        session_id: &str,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        browser_generation: u64,
        target_id: &str,
        document_id: &str,
        source_store_id: &str,
        domains: &[String],
        partition_sites: &[String],
        overwrite: bool,
        payload: &crate::runtime::browser_import_payload::BrowserImportPayload,
    ) -> Result<BrowserCookieImportOutcome, String> {
        self.require_lease(session_id)?;
        self.supervisor.import_browser_cookies(
            binding,
            browser_generation,
            target_id,
            document_id,
            source_store_id,
            domains,
            partition_sites,
            overwrite,
            payload,
        )
    }

    pub(crate) fn recover_browser_cookie_import(
        &mut self,
        session_id: &str,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        target_id: &str,
    ) -> Result<(), String> {
        self.require_lease(session_id)?;
        self.supervisor
            .recover_browser_cookie_import(binding, target_id)
    }

    fn require_lease(&self, session_id: &str) -> Result<(), String> {
        if !self.leased || self.owner_session_id.as_deref() != Some(session_id) {
            return Err(format!(
                "browser controller is not leased by Room {session_id}"
            ));
        }
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) -> Result<BrowserControllerProcessSnapshot, String> {
        let snapshot = self.supervisor.stop()?.clone();
        self.leased = false;
        Ok(snapshot)
    }
}

#[derive(Clone, Default)]
pub(crate) struct BrowserControllerProcessStore {
    ownership: Option<Arc<Mutex<StdioOwnership>>>,
    executions: cancellation::BrowserActionExecutions,
    tab_mutation_barrier: Arc<RwLock<()>>,
    tab_mutation_lanes: BrowserTabMutationLanes,
    authorizer: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
    #[cfg(test)]
    lock_wait_probe: Option<Arc<tokio::sync::Notify>>,
    app_view_replies: Arc<Mutex<()>>,
}

impl BrowserControllerProcessStore {
    pub(crate) fn with_authorizer(
        &self,
        authorizer: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Self {
        let mut store = self.clone();
        store.authorizer = Some(authorizer);
        store
    }

    pub(super) fn authorize(&self) -> Result<(), String> {
        if let Some(authorizer) = &self.authorizer {
            authorizer()?;
        }
        Ok(())
    }

    pub(crate) fn protect_observation_values(
        &self,
        room: &str,
        values: Vec<zeroize::Zeroizing<String>>,
    ) -> Result<(), String> {
        if let Some(ownership) = &self.ownership {
            #[cfg(test)]
            if let Some(probe) = &self.lock_wait_probe {
                probe.notify_one();
            }
            let mut ownership = ownership
                .lock()
                .map_err(|_| "controller supervisor lock poisoned")?;
            self.authorize()?;
            ownership
                .supervisor
                .backend
                .protected_values
                .insert(room.into(), values);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn observe_supervisor_lock_wait_for_test(
        &mut self,
        probe: Arc<tokio::sync::Notify>,
    ) {
        self.lock_wait_probe = Some(probe);
    }

    #[cfg(test)]
    pub(crate) fn new(command: impl Into<PathBuf>, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            ownership: Some(Arc::new(Mutex::new(
                BrowserControllerProcessOwnership::new(BrowserControllerProcessStdioBackend::new(
                    command, args, timeout,
                )),
            ))),
            ..Self::default()
        }
    }

    pub(crate) fn from_script(script_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            ownership: Some(Arc::new(Mutex::new(
                BrowserControllerProcessOwnership::new(
                    BrowserControllerProcessStdioBackend::from_script(script_path, timeout),
                ),
            ))),
            ..Self::default()
        }
    }

    pub(crate) fn from_environment() -> Self {
        let Some(script_path) =
            std::env::var_os(CONTROLLER_SCRIPT_ENV).filter(|path| !path.is_empty())
        else {
            return Self::default();
        };
        let timeout_ms = std::env::var(CONTROLLER_COMMAND_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONTROLLER_COMMAND_TIMEOUT_MS);
        Self::from_script(
            PathBuf::from(script_path),
            Duration::from_millis(timeout_ms),
        )
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.ownership.is_some()
    }

    pub(crate) fn acquire(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        if let Some(snapshot) = self.acquire_busy_lease(session_id)? {
            return Ok(Some(snapshot));
        }
        let _barrier = self.lock_global_tab_mutation_barrier()?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership.acquire(session_id).map(Some)
    }

    pub(crate) fn release(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let _barrier = self.lock_global_tab_mutation_barrier()?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        let snapshot = ownership.release(session_id)?;
        ownership
            .supervisor
            .backend
            .protected_values
            .remove(session_id);
        Ok(Some(snapshot))
    }

    pub(crate) fn reconcile_browser(
        &self,
        session_id: &str,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<Option<BrowserControllerReconciliation>, String> {
        self.reconcile_browser_observation(session_id, viewport, browser_bar_visible)
    }

    pub(crate) fn capture_browser_snapshot(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
    ) -> Result<Option<BrowserControllerStructuredSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let (pending, timeout) = {
            #[cfg(test)]
            if let Some(probe) = &self.lock_wait_probe {
                probe.notify_one();
            }
            let mut ownership = ownership
                .lock()
                .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
            self.authorize()?;
            ownership.require_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            supervisor.prepare_unlocked_request()?;
            let pending = supervisor
                .backend
                .begin_snapshot_read(target_id, document_id)?;
            (pending, supervisor.backend.timeout)
        };
        // Keep the Room lease check and stdin dispatch atomic, but never hold
        // ownership while waiting for a snapshot response.
        let snapshot = pending
            .wait(timeout)?
            .into_result::<BrowserControllerStructuredSnapshot>("browser.snapshot")?;
        snapshot.validate(target_id, document_id)?;
        Ok(Some(snapshot))
    }

    #[cfg(test)]
    pub(crate) fn perform_browser_action(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<Option<BrowserControllerActionResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership
            .perform_browser_action(
                session_id,
                target_id,
                document_id,
                node_ref,
                action,
                timeout_ms,
            )
            .map(Some)
    }

    pub(crate) fn wait_for_browser(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<Option<BrowserControllerCompatibilityWaitResult>, String> {
        self.wait_for_browser_observation(session_id, target_id, document_id, wait, timeout_ms)
    }

    #[cfg(test)]
    pub(crate) fn handle_browser_dialog(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<Option<BrowserControllerDialogResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership
            .handle_browser_dialog(session_id, target_id, document_id, action)
            .map(Some)
    }

    pub(crate) fn cancel_browser_download(
        &self,
        session_id: &str,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<Option<BrowserControllerDownloadCancellationResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership
            .cancel_browser_download(session_id, cancellation)
            .map(Some)
    }

    pub(crate) fn browser_artifact(
        &self,
        session_id: &str,
        request: &super::browser_artifact::BrowserArtifactRequest,
    ) -> Result<Option<super::browser_artifact::BrowserArtifactCapture>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership.browser_artifact(session_id, request).map(Some)
    }
    pub(crate) fn app_view(
        &self,
        session_id: &str,
        request: &crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<Option<serde_json::Value>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        if matches!(
            request,
            super::browser_controller_app_view::BrowserAppViewRequest::Calls
                | super::browser_controller_app_view::BrowserAppViewRequest::Respond { .. }
        ) {
            return self.app_view_bridge(session_id, request);
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership.app_view(session_id, request).map(Some)
    }

    pub(crate) fn poll_browser_events(
        &self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<Option<BrowserControllerEventBatch>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership
            .poll_browser_events(session_id, browser_generation, cursor, limit)
            .map(Some)
    }

    pub(crate) fn recover_browser_cookie_import(
        &self,
        session_id: &str,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        target_id: &str,
    ) -> Result<Option<()>, String> {
        let _barrier = self.lock_global_tab_mutation_barrier()?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership
            .recover_browser_cookie_import(session_id, binding, target_id)
            .map(Some)
    }

    pub(crate) fn shutdown(&self) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let _barrier = self.lock_global_tab_mutation_barrier()?;
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(probe) = &self.lock_wait_probe {
            probe.notify_one();
        }
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        ownership.shutdown().map(Some)
    }

    pub(crate) fn snapshot(&self) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        self.authorize()?;
        Ok(Some(ownership.supervisor.snapshot().clone()))
    }
}

impl<B: BrowserControllerProcessBackend> BrowserControllerProcessSupervisor<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            snapshot: BrowserControllerProcessSnapshot {
                state: BrowserControllerProcessState::Stopped,
                process_id: None,
                diagnostic_code: None,
                runtime_generation: 1,
                restart_count: 0,
            },
            recovery_pending: false,
            reconciled_viewport: None,
            reconciled_browser_bar_visible: false,
        }
    }

    pub(crate) fn snapshot(&self) -> &BrowserControllerProcessSnapshot {
        &self.snapshot
    }

    pub(crate) fn ensure_started(&mut self) -> Result<&BrowserControllerProcessSnapshot, String> {
        let health = self.backend.health();
        match health {
            Ok(health) if health.state == BrowserControllerProcessState::Ready => {
                self.apply_health(health);
                return Ok(&self.snapshot);
            }
            Ok(health)
                if matches!(
                    health.state,
                    BrowserControllerProcessState::Unhealthy
                        | BrowserControllerProcessState::Failed
                ) =>
            {
                self.apply_health(health);
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Ok(health)
                if health.state == BrowserControllerProcessState::Stopped
                    && self.snapshot.state != BrowserControllerProcessState::Stopped =>
            {
                self.apply_health(health);
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Err(_) => {
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Ok(health) => self.apply_health(health),
        }

        self.start()?;
        Ok(&self.snapshot)
    }

    pub(crate) fn stop(&mut self) -> Result<&BrowserControllerProcessSnapshot, String> {
        if self.snapshot.state == BrowserControllerProcessState::Stopped {
            return Ok(&self.snapshot);
        }
        self.backend.stop()?;
        self.reconciled_viewport = None;
        self.snapshot.state = BrowserControllerProcessState::Stopped;
        self.snapshot.process_id = None;
        self.snapshot.diagnostic_code = None;
        Ok(&self.snapshot)
    }

    fn reconcile_browser(
        &mut self,
        viewport: &CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<BrowserControllerReconciliation, String> {
        let process = self.ensure_started()?.clone();
        let browser = self
            .backend
            .reconcile_browser(viewport, browser_bar_visible)?;
        self.recovery_pending = false;
        self.reconciled_viewport = Some(viewport.clone());
        self.reconciled_browser_bar_visible = browser_bar_visible;
        Ok(BrowserControllerReconciliation { process, browser })
    }

    #[cfg(test)]
    fn perform_browser_action(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .perform_browser_action(target_id, document_id, node_ref, action, timeout_ms)
    }

    #[cfg(test)]
    fn handle_browser_dialog(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .handle_browser_dialog(target_id, document_id, action)
    }

    fn configure_browser_downloads(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .configure_browser_downloads(target_id, document_id)
    }

    fn cancel_browser_download(
        &mut self,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.cancel_browser_download(cancellation)
    }

    fn set_browser_permission(
        &mut self,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .set_browser_permission(target_id, document_id, permission, setting)
    }

    fn browser_artifact(
        &mut self,
        request: &super::browser_artifact::BrowserArtifactRequest,
    ) -> Result<super::browser_artifact::BrowserArtifactCapture, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.browser_artifact(request)
    }
    fn app_view(
        &mut self,
        request: &crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<serde_json::Value, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.app_view(request)
    }

    fn poll_browser_events(
        &mut self,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .poll_browser_events(browser_generation, cursor, limit)
    }

    fn import_browser_cookies(
        &mut self,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        browser_generation: u64,
        target_id: &str,
        document_id: &str,
        source_store_id: &str,
        domains: &[String],
        partition_sites: &[String],
        overwrite: bool,
        payload: &crate::runtime::browser_import_payload::BrowserImportPayload,
    ) -> Result<BrowserCookieImportOutcome, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.import_browser_cookies(
            binding,
            browser_generation,
            target_id,
            document_id,
            source_store_id,
            domains,
            partition_sites,
            overwrite,
            payload,
        )
    }

    fn recover_browser_cookie_import(
        &mut self,
        binding: &crate::transport::room_browser_controller::RoomBrowserImportBinding,
        target_id: &str,
    ) -> Result<(), String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .recover_browser_cookie_import(binding, target_id)
    }

    fn ensure_started_without_transparent_restart(&mut self) -> Result<(), String> {
        if self.recovery_pending {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.to_string());
        }
        let generation = self.snapshot.runtime_generation;
        self.ensure_started()?;
        if self.snapshot.runtime_generation != generation || self.recovery_pending {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.to_string());
        }
        Ok(())
    }

    fn start(&mut self) -> Result<(), String> {
        self.snapshot.state = BrowserControllerProcessState::Starting;
        match self.backend.start() {
            Ok(health) if health.state == BrowserControllerProcessState::Ready => {
                self.apply_health(health);
                Ok(())
            }
            Ok(health) => {
                let state = health.state;
                self.apply_health(health);
                self.snapshot.state = BrowserControllerProcessState::Failed;
                Err(format!(
                    "browser controller startup ended in {state:?} instead of Ready"
                ))
            }
            Err(error) => {
                self.snapshot.state = BrowserControllerProcessState::Failed;
                self.snapshot.process_id = None;
                self.snapshot.diagnostic_code = Some("start_failed".to_string());
                Err(error)
            }
        }
    }

    fn restart(&mut self) -> Result<(), String> {
        self.backend.stop()?;
        self.snapshot.runtime_generation = self.snapshot.runtime_generation.saturating_add(1);
        self.snapshot.restart_count = self.snapshot.restart_count.saturating_add(1);
        self.recovery_pending = true;
        self.reconciled_viewport = None;
        self.snapshot.process_id = None;
        self.start()
    }

    fn apply_health(&mut self, health: BrowserControllerProcessHealth) {
        self.snapshot.state = health.state;
        self.snapshot.process_id = health.process_id;
        self.snapshot.diagnostic_code = health.diagnostic_code;
    }

    #[cfg(test)]
    fn backend(&self) -> &B {
        &self.backend
    }
}

#[cfg(test)]
mod tests {
    // MP-08/MP-10/MP-11: controller seeds live only as long as their Room lease.
    #[test]
    fn releasing_room_drops_its_scrub_values_without_dropping_other_rooms() {
        let store = super::BrowserControllerProcessStore::new(
            "true",
            vec![],
            std::time::Duration::from_secs(1),
        );
        store
            .protect_observation_values(
                "deleted",
                vec![zeroize::Zeroizing::new("synthetic-deleted".into())],
            )
            .unwrap();
        store
            .protect_observation_values(
                "retained",
                vec![zeroize::Zeroizing::new("synthetic-retained".into())],
            )
            .unwrap();
        store.release("deleted").unwrap();
        let ownership = store.ownership.as_ref().unwrap().lock().unwrap();
        assert!(!ownership
            .supervisor
            .backend
            .protected_values
            .contains_key("deleted"));
        assert_eq!(
            ownership.supervisor.backend.protected_values["retained"].len(),
            1
        );
    }

    use std::collections::VecDeque;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        BrowserControllerBrowserSnapshot, BrowserControllerProcessBackend,
        BrowserControllerProcessHealth, BrowserControllerProcessOwnership,
        BrowserControllerProcessState, BrowserControllerProcessStdioBackend,
        BrowserControllerProcessStore, BrowserControllerProcessSupervisor,
        CONTROLLER_RESTARTED_BEFORE_OPERATION,
    };
    use crate::runtime::browser_controller_action::{BrowserDialogAction, BrowserLocatorAction};
    use crate::session::CanonicalViewport;

    const HEALTHY_TEST_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct FakeBackend {
        health: VecDeque<Result<BrowserControllerProcessHealth, String>>,
        starts: VecDeque<Result<BrowserControllerProcessHealth, String>>,
        start_count: usize,
        stop_count: usize,
    }

    impl BrowserControllerProcessBackend for FakeBackend {
        fn health(&mut self) -> Result<BrowserControllerProcessHealth, String> {
            self.health
                .pop_front()
                .expect("test must provide a health result")
        }

        fn start(&mut self) -> Result<BrowserControllerProcessHealth, String> {
            self.start_count += 1;
            self.starts
                .pop_front()
                .expect("test must provide a start result")
        }

        fn stop(&mut self) -> Result<(), String> {
            self.stop_count += 1;
            Ok(())
        }
    }

    fn health(
        state: BrowserControllerProcessState,
        process_id: Option<u32>,
    ) -> BrowserControllerProcessHealth {
        BrowserControllerProcessHealth {
            state,
            process_id,
            diagnostic_code: None,
        }
    }

    #[test]
    fn ensure_started_launches_a_stopped_controller() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(41))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller starts");

        assert_eq!(snapshot.state, BrowserControllerProcessState::Ready);
        assert_eq!(snapshot.process_id, Some(41));
        assert_eq!(snapshot.runtime_generation, 1);
        assert_eq!(snapshot.restart_count, 0);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 0);
    }

    #[test]
    fn ensure_started_reuses_a_healthy_controller() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(42))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller is reused");

        assert_eq!(snapshot.process_id, Some(42));
        assert_eq!(snapshot.runtime_generation, 1);
        assert_eq!(supervisor.backend().start_count, 0);
        assert_eq!(supervisor.backend().stop_count, 0);
    }

    #[test]
    fn ensure_started_restarts_an_unhealthy_controller() {
        let mut backend = FakeBackend::default();
        backend.health.push_back(Ok(BrowserControllerProcessHealth {
            state: BrowserControllerProcessState::Unhealthy,
            process_id: Some(43),
            diagnostic_code: Some("health_timeout".to_string()),
        }));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller restarts");

        assert_eq!(snapshot.process_id, Some(44));
        assert_eq!(snapshot.runtime_generation, 2);
        assert_eq!(snapshot.restart_count, 1);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn mutation_does_not_run_after_an_implicit_controller_restart() {
        let mut backend = FakeBackend::default();
        backend.health.push_back(Ok(BrowserControllerProcessHealth {
            state: BrowserControllerProcessState::Unhealthy,
            process_id: Some(43),
            diagnostic_code: Some("health_timeout".to_string()),
        }));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("mutation must wait for recovery reconciliation");

        assert_eq!(
            error,
            "browser controller restarted before the operation; reconcile and retry with fresh references"
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 2);
        assert_eq!(supervisor.snapshot().restart_count, 1);

        let repeated_error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("the restarted controller stays fenced until reconciliation");

        assert_eq!(repeated_error, error);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn mutation_does_not_run_when_a_ready_controller_disappears() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(43))));
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        supervisor.ensure_started().expect("controller starts");

        let error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("a disappeared ready controller requires reconciliation");

        assert_eq!(error, CONTROLLER_RESTARTED_BEFORE_OPERATION);
        assert_eq!(supervisor.snapshot().runtime_generation, 2);
        assert_eq!(supervisor.snapshot().restart_count, 1);
        assert_eq!(supervisor.backend().start_count, 2);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn failed_start_is_reported_without_claiming_ready() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Err("controller did not become healthy".to_string()));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let error = supervisor.ensure_started().expect_err("startup must fail");

        assert_eq!(error, "controller did not become healthy");
        assert_eq!(
            supervisor.snapshot().state,
            BrowserControllerProcessState::Failed
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 1);
    }

    #[test]
    fn stop_is_idempotent_and_does_not_advance_generation() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(45))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        supervisor.ensure_started().expect("controller is running");

        supervisor.stop().expect("first stop succeeds");
        supervisor.stop().expect("second stop succeeds");

        assert_eq!(
            supervisor.snapshot().state,
            BrowserControllerProcessState::Stopped
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn app_open_reports_the_environment_owner_and_how_to_view_it() {
        let mut ownership = BrowserControllerProcessOwnership::new(FakeBackend::default());
        ownership.owner_session_id = Some("room-1".to_owned());
        ownership.leased = true;
        let error = ownership
            .app_view(
                "room-2",
                &crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Calls,
            )
            .unwrap_err();
        assert!(error.contains("belongs to Room room-1, not Room room-2"));
        assert!(error.contains("/app open <installation-id>"));
        assert!(error.contains("/room view"));
        assert_eq!(ownership.owner_session_id.as_deref(), Some("room-1"));
        assert!(
            ownership.leased,
            "failed open must leave the owner's browser alone"
        );
    }

    #[test]
    fn app_open_in_an_unstarted_environment_explains_how_to_start_it() {
        let mut ownership = BrowserControllerProcessOwnership::new(FakeBackend::default());
        let error = ownership
            .app_view(
                "room-1",
                &crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Calls,
            )
            .unwrap_err();
        assert!(error.contains("not started"));
        assert!(error.contains("/room start"));
    }

    #[test]
    fn physical_browser_binding_survives_release_and_shutdown() {
        let tool = TestTool::new(responsive_controller_script());
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        assert!(store.acquire(" ").is_err());
        let first = store.acquire("room-1").unwrap().unwrap();
        assert_eq!(
            store.acquire("room-1").unwrap().unwrap().process_id,
            first.process_id
        );
        assert!(store.acquire("room-2").is_err());
        assert_eq!(
            store.release("room-2").unwrap().unwrap().process_id,
            first.process_id
        );
        assert_eq!(
            store.release("room-1").unwrap().unwrap().state,
            BrowserControllerProcessState::Stopped
        );
        assert!(store.acquire("room-2").is_err());
        store.acquire("room-1").expect("owner restarts");
        store.shutdown().expect("controller shuts down");
        assert!(store.acquire("room-2").is_err());
        store
            .acquire("room-1")
            .expect("shutdown retains owner binding");
        store.shutdown().expect("clean up controller");
    }

    #[test]
    fn concurrent_rooms_admit_only_one_physical_browser_owner() {
        let tool = TestTool::new(responsive_controller_script());
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let barrier = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                store.acquire("room-1")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                store.acquire("room-2")
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(
            first.is_ok(),
            second.is_ok(),
            "exactly one Room must be admitted"
        );
        let (owner, rejected, snapshot) = match (first, second) {
            (Ok(Some(snapshot)), Err(_)) => ("room-1", "room-2", snapshot),
            (Err(_), Ok(Some(snapshot))) => ("room-2", "room-1", snapshot),
            results => panic!("unexpected admission results: {results:?}"),
        };
        store
            .release(rejected)
            .expect("rejected Room release is harmless");
        assert_eq!(
            store.acquire(owner).unwrap().unwrap().process_id,
            snapshot.process_id
        );
        store.shutdown().expect("clean up controller");
    }

    #[test]
    fn failed_controller_start_keeps_the_physical_browser_owner() {
        let tool = TestTool::new("#!/bin/sh\nexit 1\n");
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        assert!(store.acquire("room-1").is_err());
        assert_eq!(
            store.acquire("room-2").unwrap_err(),
            "browser controller is bound to another Room"
        );
        assert_eq!(
            store.release("room-1").unwrap().unwrap().state,
            BrowserControllerProcessState::Stopped
        );
        assert_eq!(
            store.acquire("room-2").unwrap_err(),
            "browser controller is bound to another Room"
        );
        store.shutdown().expect("clean up failed controller");
    }

    pub(super) struct TestTool {
        pub(super) root: PathBuf,
        path: PathBuf,
    }

    impl TestTool {
        pub(super) fn new(script: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "chariox-browser-controller-process-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("create test tool root");
            let source = root.join("controller-tool.source");
            fs::write(&source, script).expect("write test tool source");
            // Install the executable from a child process: a writable fd held
            // by this multi-threaded test process could leak into another
            // test's fork and make exec fail with ETXTBSY.
            let path = root.join("controller-tool.sh");
            let installed = std::process::Command::new("install")
                .args(["-m", "700"])
                .arg(&source)
                .arg(&path)
                .status()
                .expect("run install for test tool");
            assert!(installed.success(), "install test tool: {installed}");
            Self { root, path }
        }

        pub(super) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTool {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn responsive_controller_script() -> &'static str {
        "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n"
    }

    #[test]
    fn stdio_backend_starts_reports_health_and_stops() {
        let tool = TestTool::new(responsive_controller_script());
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        assert_eq!(
            backend.health().expect("stopped health").state,
            BrowserControllerProcessState::Stopped
        );
        let started = backend.start().expect("controller starts");
        let health = backend.health().expect("health response parses");

        assert_eq!(started.state, BrowserControllerProcessState::Ready);
        assert_eq!(health.state, BrowserControllerProcessState::Ready);
        assert_eq!(health.process_id, started.process_id);
        assert_eq!(health.diagnostic_code, None);
        backend.stop().expect("controller stops");
        assert_eq!(
            backend.health().expect("stopped health").state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[test]
    fn stdio_backend_skips_a_stale_response_before_the_matching_reply() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":0,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$$\"; printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let started = backend
            .start()
            .expect("a stale response should not mask the matching health reply");

        assert_eq!(started.state, BrowserControllerProcessState::Ready);
        backend.stop().expect("controller stops");
    }

    #[test]
    fn room_lease_reconciles_browser_tabs_and_canonical_viewport_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.reconcile\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"tabs\":[{\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"url\":\"https://a.test\",\"title\":\"A\"}],\"focused_target_id\":\"target-a\",\"resource_inventory\":{\"browser_ids\":[\"browser-pid-41\"],\"profile_ids\":[\"profile-sha256-41\"]},\"viewport\":{\"css_width\":1280,\"css_height\":720,\"device_scale_factor\":1,\"desktop_pixel_width\":1280,\"desktop_pixel_height\":720}}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();

        assert!(store.reconcile_browser("room-1", &viewport, false).is_err());
        store.acquire("room-1").expect("Room acquires controller");
        let reconciliation = store
            .reconcile_browser("room-1", &viewport, false)
            .expect("browser reconciles")
            .expect("controller is enabled");

        assert_eq!(
            reconciliation.process.state,
            BrowserControllerProcessState::Ready
        );
        assert_eq!(reconciliation.browser.browser_generation, 1);
        assert_eq!(reconciliation.browser.tabs[0].target_id, "target-a");
        assert_eq!(
            reconciliation.browser.focused_target_id.as_deref(),
            Some("target-a")
        );
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn worker_resource_inventory_rejects_zero_duplicate_and_multiple_observations() {
        let viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
        for (browser_ids, profile_ids) in [
            (vec![], vec!["profile-sha256-41"]),
            (
                vec!["browser-pid-41", "browser-pid-41"],
                vec!["profile-sha256-41", "profile-sha256-41"],
            ),
            (
                vec!["browser-pid-41", "browser-pid-42"],
                vec!["profile-sha256-41", "profile-sha256-42"],
            ),
        ] {
            let snapshot: BrowserControllerBrowserSnapshot =
                serde_json::from_value(serde_json::json!({
                    "browser_generation": 1,
                    "event_cursor": 1,
                    "tabs": [],
                    "focused_target_id": null,
                    "resource_inventory": {
                        "browser_ids": browser_ids,
                        "profile_ids": profile_ids,
                    },
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 720,
                        "device_scale_factor": 1,
                        "desktop_pixel_width": 1280,
                        "desktop_pixel_height": 720,
                    },
                }))
                .expect("resource inventory response shape parses");
            assert!(snapshot.validate(&viewport).is_err());
        }
    }

    #[test]
    fn room_snapshot_reads_overlap_and_match_out_of_order_responses() {
        // Neither snapshot can finish until the controller receives both.
        // Reply in reverse order to catch a shared response receiver as well
        // as an ownership lock held across the first RPC.
        let tool = TestTool::new(
            r#"#!/bin/sh
set -eu
first_id=
snapshot() {
  printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"%s","document_id":"loader-a","snapshot_revision":1,"accessibility_nodes":[],"dom_nodes":[]}}\n' "$1" "$2"
}
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.snapshot"'*)
      if [ -z "$first_id" ]; then
        first_id=$id
        touch "$1"
      else
        snapshot "$id" target-second
        snapshot "$first_id" target-first
      fi ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
  esac
done
"#,
        );
        let started = tool.root.join("first-snapshot-started");
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            vec![started.display().to_string()],
            Duration::from_secs(3),
        );
        store.acquire("room-1").expect("acquire controller");
        let first_store = store.clone();
        let first = std::thread::spawn(move || {
            first_store.capture_browser_snapshot("room-1", "target-first", "loader-a")
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !started.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(started.exists(), "first snapshot must reach the controller");
        let second_store = store.clone();
        let second = std::thread::spawn(move || {
            second_store.capture_browser_snapshot("room-1", "target-second", "loader-a")
        });
        let first = first.join().expect("first reader joins");
        let second = second.join().expect("second reader joins");
        store.release("room-1").expect("release controller");
        assert_eq!(
            first
                .expect("first snapshot must overlap")
                .unwrap()
                .target_id,
            "target-first"
        );
        assert_eq!(
            second
                .expect("second response must match")
                .unwrap()
                .target_id,
            "target-second"
        );
    }

    #[test]
    fn room_lease_captures_a_document_bound_structured_snapshot_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.snapshot\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"snapshot_revision\":1,\"accessibility_nodes\":[{\"node_ref\":\"backend:103\",\"parent_ref\":null,\"child_refs\":[],\"role\":\"button\",\"name\":\"Save\",\"description\":\"\",\"value\":\"\",\"ignored\":false,\"disabled\":false,\"focused\":true}],\"dom_documents\":[{\"document_index\":0,\"url\":\"https://a.test\",\"owner_node_ref\":null}],\"dom_nodes\":[{\"node_ref\":\"backend:103\",\"parent_ref\":\"backend:102\",\"document_index\":0,\"node_type\":1,\"node_name\":\"BUTTON\",\"text\":\"\",\"attributes\":{\"id\":\"save\"},\"bounds\":{\"x\":10,\"y\":20,\"width\":100,\"height\":30}}]}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        store.acquire("room-1").expect("Room acquires controller");

        let snapshot = store
            .capture_browser_snapshot("room-1", "target-a", "loader-a")
            .expect("structured snapshot succeeds")
            .expect("controller is enabled");

        assert_eq!(snapshot.browser_generation, 1);
        assert_eq!(snapshot.snapshot_revision, 1);
        assert_eq!(snapshot.accessibility_nodes[0].node_ref, "backend:103");
        assert_eq!(snapshot.dom_documents[0].document_index, 0);
        assert_eq!(snapshot.dom_nodes[0].attributes["id"], "save");
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn room_lease_performs_a_bounded_locator_action_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.action\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"action_kind\":\"click\",\"dialog_opened\":true,\"attempts\":2,\"elapsed_ms\":50}}\\n' \"$id\" ;;\n    *'\"method\":\"browser.dialog\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"action\":\"dismiss\"}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        store.acquire("room-1").expect("Room acquires controller");

        let result = store
            .perform_browser_action(
                "room-1",
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                500,
            )
            .expect("locator action succeeds")
            .expect("controller is enabled");

        assert_eq!(result.action_kind, "click");
        assert!(result.dialog_opened);
        assert_eq!(result.attempts, 2);
        assert_eq!(result.elapsed_ms, 50);
        let dialog = store
            .handle_browser_dialog(
                "room-1",
                "target-a",
                "loader-a",
                &BrowserDialogAction::Dismiss,
            )
            .expect("dialog action succeeds")
            .expect("controller is enabled");
        assert_eq!(dialog.action, "dismiss");
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn stdio_supervisor_restarts_a_crashed_controller_with_a_new_process() {
        let tool = TestTool::new(responsive_controller_script());
        let backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        let first = supervisor
            .ensure_started()
            .expect("first controller starts")
            .clone();
        let first_process_id = first.process_id.expect("first process id");

        assert!(first_process_id > 1 && first_process_id <= i32::MAX as u32);
        let kill_result = unsafe { libc::kill(first_process_id as i32, libc::SIGKILL) };
        assert_eq!(kill_result, 0, "test controller should be killable");
        let restarted = supervisor
            .ensure_started()
            .expect("crashed controller restarts")
            .clone();

        assert_eq!(restarted.state, BrowserControllerProcessState::Ready);
        assert_ne!(restarted.process_id, Some(first_process_id));
        assert_eq!(restarted.runtime_generation, 2);
        assert_eq!(restarted.restart_count, 1);
        supervisor.stop().expect("restarted controller stops");
    }

    #[test]
    fn stdio_backend_reports_early_exit_without_claiming_ready() {
        // MP-08/MP-10: exit after consuming health so the fixture exercises
        // response EOF, rather than racing the request write with a broken pipe.
        let tool = TestTool::new("#!/bin/sh\nIFS= read -r request\nexit 9\n");
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let error = backend.start().expect_err("early exit must fail");

        assert!(
            error.contains("exited during `health`"),
            "unexpected early-exit diagnostic: {error}"
        );
    }

    #[test]
    fn stdio_backend_rejects_a_process_identity_mismatch_and_reaps_the_child() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nIFS= read -r request\nid=${request#*:}\nid=${id%%,*}\nprintf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":1,\"diagnostic_code\":null}}\\n' \"$id\"\nexec sleep 30\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let error = backend.start().expect_err("foreign process id must fail");

        assert!(error.contains("expected"));
        assert_eq!(
            backend
                .health()
                .expect("mismatched controller was reaped")
                .state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[test]
    fn stdio_backend_kills_a_controller_that_does_not_answer_health() {
        let tool = TestTool::new("#!/bin/sh\nexec sleep 30\n");
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            Duration::from_millis(25),
        );

        let started = std::time::Instant::now();
        let error = backend.start().expect_err("health request must time out");

        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            backend
                .health()
                .expect("timed out process was reaped")
                .state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn md4_stop_retains_descendant_ownership_after_controller_exit() {
        fn alive(pid: u32) -> bool {
            fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .is_some_and(|s| {
                    s.rsplit_once(')')
                        .is_some_and(|(_, tail)| !tail.trim_start().starts_with('Z'))
                })
        }
        for crash in [true, false] {
            let tool = TestTool::new(
                r#"#!/bin/sh
set -eu
sleep 30 &
printf '%s' "$!" > "$0.survivor"
while IFS= read -r request; do
id=${request#*:}; id=${id%%,*}
case "$request" in
*'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
*'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
esac
done
"#,
            );
            let mut backend = BrowserControllerProcessStdioBackend::new(
                tool.path(),
                Vec::new(),
                Duration::from_millis(250),
            );
            backend.start().unwrap();
            let survivor: u32 = fs::read_to_string(tool.path().with_extension("sh.survivor"))
                .unwrap()
                .parse()
                .unwrap();
            assert!(survivor > 1 && alive(survivor));
            // Retain an independent ownership witness only to clean up a RED run.
            let process = backend.process.as_mut().unwrap();
            process.owned_group.refresh();
            let mut cleanup = super::owned_process_group::OwnedProcessGroup::new(
                process.child.lock().unwrap().id(),
            );
            if crash {
                let mut child = process.child.lock().unwrap();
                assert!(child.id() > 1);
                child.kill().unwrap();
                child.wait().unwrap();
            }
            backend.stop().unwrap(); // No health()/take_exited_process() before Stop.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while alive(survivor) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let leaked = alive(survivor);
            cleanup.signal();
            assert!(!leaked, "MD-4: Stop/shutdown must reap the recorded Chrome survivor (controller crash={crash})");
            backend.start().unwrap();
            backend.stop().unwrap();
        }
    }

    #[test]
    fn stdio_backend_bounds_shutdown_and_kills_the_process_group() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nIFS= read -r request\nid=${request#*:}\nid=${id%%,*}\nprintf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\"\nIFS= read -r request\nsleep 30 &\nwait\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            Duration::from_secs(2),
        );
        backend.start().expect("controller starts");

        let started = std::time::Instant::now();
        backend.stop().expect("bounded forced stop succeeds");

        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            backend.health().expect("forced process was reaped").state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mp10_host_encoder_environment_crosses_the_real_spawn_boundary() {
        // MP-08/MP-10/MP-11: isolate environment changes in a test subprocess.
        if std::env::var("CHARIOX_TEST_DISPLAY_ENV_CHILD").as_deref() != Ok("1") {
            let name = format!(
                "{}::mp10_host_encoder_environment_crosses_the_real_spawn_boundary",
                module_path!().split_once("::").unwrap().1
            );
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"])
                .env("CHARIOX_TEST_DISPLAY_ENV_CHILD", "1")
                .env("CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER", "libopenh264")
                .env(
                    "CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER",
                    "/fixture/openh264.so",
                )
                .env(
                    "CHARIOX_BROWSER_DISPLAY_OPENH264",
                    "/fixture/libopenh264.so.8",
                )
                .env("CHARIOX_BROWSER_DISPLAY_LIBYUV", "/fixture/libyuv.so")
                .env("CHARIOX_TEST_CONTROL_SECRET", "synthetic")
                .status()
                .unwrap();
            assert!(
                status.success(),
                "MP-10: host discarded explicit encoder/converter configuration"
            );
            return;
        }
        let tool = TestTool::new(
            r#"#!/bin/sh
set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
 *'"method":"host.browser"'*) printf '{"id":%s,"ok":true,"result":{"encoder":"%s","adapter":"%s","converter":"%s","native_encoder":"%s","control_secret_present":%s}}\n' "$id" "${CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER-}" "${CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER-}" "${CHARIOX_BROWSER_DISPLAY_LIBYUV-}" "${CHARIOX_BROWSER_DISPLAY_OPENH264-}" "${CHARIOX_TEST_CONTROL_SECRET+true}" | sed 's/:}/:false}/' ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
 esac
done
"#,
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            Duration::from_secs(2),
        )
        .for_host();
        backend.start().unwrap();
        let result = backend.host_request("host.browser", serde_json::json!({"op":"probe"}));
        backend.stop().unwrap();
        assert_eq!(
            result.unwrap(),
            serde_json::json!({
                "encoder":"libopenh264", "adapter":"/fixture/openh264.so",
                "converter":"/fixture/libyuv.so", "native_encoder":"/fixture/libopenh264.so.8", "control_secret_present":false
            })
        );
    }
}
