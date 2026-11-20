//! Room-scoped completion notices and the native action for completed downloads.
use super::{kernel_browser_runtime::host_error, KernelRuntimeState};
use crate::error::DaemonError;
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

const MAX_WAIT_MS: u32 = 10_000;

impl KernelRuntimeState {
    pub(super) fn room_downloads_owner(
        &self,
        caller: &KernelCommand,
        session_id: &str,
    ) -> Result<String, DaemonError> {
        let (user, _) = self.kernel_browser_terminal_context(caller)?;
        let caller_user = command_caller_user_id(caller);
        let room_owner = self.owned.session_store.session_owner_user_id(session_id);
        if !room_owner.is_some_and(|owner| self.same_owner(&owner, &caller_user)) {
            return Err(DaemonError::SessionAccessDenied {
                session_id: session_id.to_string(),
                user_id: caller_user,
            });
        }
        Ok(user)
    }

    pub(super) async fn room_downloads_wait(
        &self,
        caller: &KernelCommand,
        session_id: &str,
        after: String,
        wait_ms: u32,
    ) -> Result<Value, DaemonError> {
        if wait_ms > MAX_WAIT_MS || after.len() > 26_000 {
            return Err(host_error("Invalid browser download wait".into()));
        }
        let user = self.room_downloads_owner(caller, session_id)?;
        let host = self.owned.kernel_browser_host.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms.into());
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let params = json!({
                "session_id": session_id, "after": after, "wait_ms": remaining.as_millis() as u64,
            });
            let (host, user) = (host.clone(), user.clone());
            let reply = tokio::task::spawn_blocking(move || host.downloads_wait(&user, &params))
                .await
                .map_err(|_| host_error("Browser download wait interrupted".into()))?
                .map_err(host_error)?;
            self.room_downloads_owner(caller, session_id)?;
            if let Some(reply) = reply {
                return Ok(reply);
            }
            if remaining.is_zero() {
                return Ok(json!({"downloads": [], "revision": ""}));
            }
            // A Room without a running browser stays subscribed without starting Chrome.
            tokio::time::sleep(remaining.min(Duration::from_millis(250))).await;
        }
    }

    pub(super) async fn room_download_reveal(
        &self,
        caller: &KernelCommand,
        session_id: &str,
        download_id: &str,
    ) -> Result<Value, DaemonError> {
        let reply = self
            .room_downloads_wait(caller, session_id, String::new(), 0)
            .await?;
        let path = reply["downloads"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|download| download["download_id"] == download_id)
            .and_then(|download| download["path"].as_str())
            .map(Path::new)
            .ok_or_else(|| host_error("Download is no longer available in this Room".into()))?;
        let native = caller.same_computer && reveal_in_file_manager(path).await;
        Ok(json!({"native": native}))
    }
}

async fn reveal_in_file_manager(path: &Path) -> bool {
    if !tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|entry| entry.is_file())
    {
        return false;
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = tokio::process::Command::new("/usr/bin/open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let Ok(url) = url::Url::from_file_path(path) else {
            return false;
        };
        let items = format!(
            "[{}]",
            serde_json::to_string(url.as_str()).expect("file URL serializes")
        );
        let mut command = tokio::process::Command::new("gdbus");
        command.args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
            &items,
            "",
        ]);
        command
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::process::Stdio;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(3), command.status())
            .await
            .is_ok_and(|result| result.is_ok_and(|status| status.success()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    false
}
