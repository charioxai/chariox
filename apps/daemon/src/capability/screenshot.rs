use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenshotStatus {
    Captured,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureScreenshotRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub artifact_root: PathBuf,
}

impl CaptureScreenshotRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        artifact_root: PathBuf,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            artifact_root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureScreenshotResult {
    pub session_id: String,
    pub status: ScreenshotStatus,
    pub artifact_path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ScreenshotCapabilityService;

impl ScreenshotCapabilityService {
    pub fn new() -> Self {
        Self
    }

    pub fn capture(
        &self,
        request: CaptureScreenshotRequest,
    ) -> Result<CaptureScreenshotResult, DaemonError> {
        let _guard = crate::env_lock::lock();
        if std::env::var("ARROBA_SCREENSHOT_DISABLE").as_deref() == Ok("1") {
            return Ok(CaptureScreenshotResult {
                session_id: request.session_id,
                status: ScreenshotStatus::Unavailable,
                artifact_path: None,
                message: "Screenshot capture disabled by environment override".to_string(),
            });
        }

        std::fs::create_dir_all(&request.artifact_root).map_err(|error| {
            DaemonError::ScreenshotCapabilityFailed {
                session_id: request.session_id.clone(),
                message: error.to_string(),
            }
        })?;

        let artifact_path = request
            .artifact_root
            .join(format!("screenshot-{}.png", timestamp_ms()));

        match try_capture_to(&artifact_path) {
            Ok(true) => Ok(CaptureScreenshotResult {
                session_id: request.session_id,
                status: ScreenshotStatus::Captured,
                artifact_path: Some(artifact_path),
                message: "Screenshot captured".to_string(),
            }),
            Ok(false) => Ok(CaptureScreenshotResult {
                session_id: request.session_id,
                status: ScreenshotStatus::Unavailable,
                artifact_path: None,
                message: "No supported screenshot backend is available on this host".to_string(),
            }),
            Err(message) => Err(DaemonError::ScreenshotCapabilityFailed {
                session_id: request.session_id,
                message,
            }),
        }
    }
}

fn try_capture_to(path: &Path) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        run_optional_command(
            "screencapture",
            &["-x".to_string(), path.display().to_string()],
        )
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(true) = run_optional_command("grim", &[path.display().to_string()]) {
            return Ok(true);
        }
        if let Ok(true) = run_optional_command(
            "gnome-screenshot",
            &["-f".to_string(), path.display().to_string()],
        ) {
            return Ok(true);
        }
        if let Ok(true) = run_optional_command("scrot", &[path.display().to_string()]) {
            return Ok(true);
        }
        Ok(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Ok(false)
    }
}

fn run_optional_command(command: &str, args: &[String]) -> Result<bool, String> {
    match Command::new(command).args(args).output() {
        Ok(output) if output.status.success() => Ok(true),
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{CaptureScreenshotRequest, ScreenshotCapabilityService, ScreenshotStatus};

    #[test]
    fn returns_unavailable_when_disabled() {
        let _guard = crate::env_lock::lock();
        std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
        let result = ScreenshotCapabilityService::new()
            .capture(CaptureScreenshotRequest::new(
                "session-1",
                "attachment-1",
                std::env::temp_dir().join("arroba-screenshot-test"),
            ))
            .expect("capture should return structured unavailable result");
        std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

        assert_eq!(result.status, ScreenshotStatus::Unavailable);
        assert!(result.artifact_path.is_none());
    }
}
