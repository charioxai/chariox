//! Human-owned visible pixel capture. Room reads retain the worker's Vault
//! fence; host tabs use CDP compositor pixels and trusted protection bounds.
use super::KernelRuntimeState;
use crate::{
    error::DaemonError,
    local::*,
    runtime::command::{command_caller_user_id, KernelCommand},
};
use base64::Engine as _;
use sha2::{Digest, Sha256};

mod raster;
use raster::{crop_png, CaptureMask};
static CAPTURE_BUDGET: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(super) fn capture_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "screenshot.capture",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn capture_visible_region(
        &self,
        command: &KernelCommand,
        request: CaptureVisibleRegionRequest,
    ) -> Result<VisibleRegionCapture, DaemonError> {
        if !command.is_terminal_caller() {
            return Err(DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::NotGranted,
            });
        }
        if request.capture_id.is_empty()
            || request.capture_id.len() > 64
            || !request
                .capture_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(capture_error("Invalid capture ID"));
        }
        raster::validate_region(&request.region)?;
        let _capture_budget = CAPTURE_BUDGET.try_acquire().map_err(|_| {
            capture_error("Capture busy; retry after the current selection finishes")
        })?;
        let owner = command_caller_user_id(command);
        let mut masks = Vec::new();
        let bytes = match &request.surface {
            ScreenshotSurface::Room {
                session_id,
                attachment_id,
                runtime_generation,
                viewport_revision,
            } => {
                let slice = self
                    .authorize_room_screenshot(&command.caller, session_id, attachment_id)
                    .await?;
                let before = self
                    .room_environment_snapshot(session_id)
                    .map_err(|_| capture_error("Room unavailable"))?;
                if before.runtime_generation != *runtime_generation
                    || before.viewport.revision != *viewport_revision
                    || before.viewport.desktop_pixel_width != request.region.frame_width
                    || before.viewport.desktop_pixel_height != request.region.frame_height
                {
                    return Err(capture_error("Stale Room frame; refresh before capturing"));
                }
                let artifact = self
                    .capture_room_environment_screenshot(
                        &command.caller,
                        CaptureRoomEnvironmentScreenshotRequest {
                            session_id: session_id.clone(),
                            attachment_id: attachment_id.clone(),
                        },
                    )
                    .await?;
                let bytes = self
                    .read_complete_room_screenshot(session_id, &slice, &artifact, 16 * 1024 * 1024)
                    .await?;
                // The protected worker read also checks the observation epoch and
                // revision on each chunk. Viewport changes invalidate this crop.
                let after = self
                    .room_environment_snapshot(session_id)
                    .map_err(|_| capture_error("Room unavailable"))?;
                if before.runtime_generation != after.runtime_generation
                    || before.viewport != after.viewport
                {
                    return Err(capture_error("Room changed during capture"));
                }
                bytes
            }
            _ => {
                let (tab_id, generation) = match &request.surface {
                    ScreenshotSurface::KernelBrowser { tab_id, generation } => {
                        (tab_id.clone(), *generation)
                    }
                    ScreenshotSurface::UserAppView {
                        view_id,
                        generation,
                    } => {
                        #[cfg(any(
                            target_os = "macos",
                            all(target_os = "linux", target_env = "gnu")
                        ))]
                        {
                            let (_, view) =
                                self.app_control().user_views().get(&owner, view_id).ok_or(
                                    DaemonError::UserDomainRefused {
                                        reason: crate::error::UserDomainRefusalReason::NotGranted,
                                    },
                                )?;
                            let browser = view.browser.ok_or_else(|| {
                                capture_error(
                                    "Native App views are captured by the trusted client renderer",
                                )
                            })?;
                            if browser.generation != *generation {
                                return Err(DaemonError::UserDomainRefused {
                                    reason: crate::error::UserDomainRefusalReason::StaleEpoch,
                                });
                            }
                            (browser.tab_id, browser.generation)
                        }
                        #[cfg(not(any(
                            target_os = "macos",
                            all(target_os = "linux", target_env = "gnu")
                        )))]
                        {
                            let _ = (view_id, generation);
                            return Err(capture_error("App host unavailable"));
                        }
                    }
                    _ => unreachable!(),
                };
                if tab_id.is_empty() || tab_id.len() > 128 || generation == 0 {
                    return Err(capture_error("Invalid host identity"));
                }
                let frame = self
                    .kernel_browser_display_request(
                        command,
                        super::KernelBrowserDisplayRequest::CaptureRegion { tab_id, generation },
                    )
                    .await?;
                if frame.get("generation").and_then(|v| v.as_u64()) != Some(generation)
                    || frame.get("mime_type").and_then(|v| v.as_str()) != Some("image/png")
                {
                    return Err(capture_error("Host returned a stale or invalid frame"));
                }
                masks = serde_json::from_value::<Vec<CaptureMask>>(
                    frame
                        .get("protected_regions")
                        .cloned()
                        .ok_or_else(|| capture_error("Host protection unavailable"))?,
                )
                .map_err(|_| capture_error("Invalid host protection bounds"))?;
                let encoded = frame
                    .get("data_base64")
                    .and_then(|v| v.as_str())
                    .filter(|v| v.len() <= 24 * 1024 * 1024)
                    .ok_or_else(|| capture_error("Host frame exceeds limit"))?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| capture_error("Invalid host frame encoding"))?;
                // Close/revocation wins over an in-flight App capture.
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                if let ScreenshotSurface::UserAppView {
                    view_id,
                    generation,
                } = &request.surface
                {
                    let owner = command_caller_user_id(command);
                    if !self
                        .app_control()
                        .user_views()
                        .get(&owner, view_id)
                        .is_some_and(|(_, v)| {
                            v.browser.is_some_and(|b| b.generation == *generation)
                        })
                    {
                        return Err(capture_error("App closed during capture"));
                    }
                }
                bytes
            }
        };
        let region = request.region;
        let cropped = tokio::task::spawn_blocking(move || crop_png(&bytes, &region, &masks))
            .await
            .map_err(|_| capture_error("Image crop task failed"))??;
        let captured_at_ms = crate::session::unix_epoch_ms();
        Ok(VisibleRegionCapture {
            capture_id: request.capture_id,
            surface: request.surface,
            captured_at_ms,
            width: cropped.width,
            height: cropped.height,
            media_type: "image/png".into(),
            display_name: format!("chariox-capture-{captured_at_ms}.png"),
            sha256: format!("{:x}", Sha256::digest(&cropped.bytes)),
            data_base64: base64::engine::general_purpose::STANDARD.encode(cropped.bytes),
        })
    }
}
