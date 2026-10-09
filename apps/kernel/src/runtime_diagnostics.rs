//! MP-07/MP-08/MP-10/MP-11: opt-in, payload-free campaign diagnostics.
//! This journal is observation only. It never owns runtime or update state.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
static WRITER: Mutex<Option<File>> = Mutex::new(None);

#[derive(Clone, Copy)]
pub(crate) enum Event {
    PromptDispatch,
    ProviderDispatchStart,
    ProviderDispatchReturned,
    ProviderDispatchFailed,
    HeartbeatSent,
    HeartbeatFailed,
    UpdatePoll,
    UpdatePollFailed,
    UpdateUnitRunning,
    UpdateRecovery,
    UpdatePending,
    UpdateApplied,
    UpdateFailed,
    UpdateCloudAcknowledged,
    UpdateDownload,
    UpdateDownloaded,
    UpdateUnitStarting,
    UpdateUnitStarted,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Self::PromptDispatch => "prompt_dispatch",
            Self::ProviderDispatchStart => "provider_dispatch_start",
            Self::ProviderDispatchReturned => "provider_dispatch_returned",
            Self::ProviderDispatchFailed => "provider_dispatch_failed",
            Self::HeartbeatSent => "heartbeat_sent",
            Self::HeartbeatFailed => "heartbeat_failed",
            Self::UpdatePoll => "update_poll",
            Self::UpdatePollFailed => "update_poll_failed",
            Self::UpdateUnitRunning => "update_unit_running",
            Self::UpdateRecovery => "update_recovery",
            Self::UpdatePending => "update_pending",
            Self::UpdateApplied => "update_applied",
            Self::UpdateFailed => "update_failed",
            Self::UpdateCloudAcknowledged => "update_cloud_acknowledged",
            Self::UpdateDownload => "update_download",
            Self::UpdateDownloaded => "update_downloaded",
            Self::UpdateUnitStarting => "update_unit_starting",
            Self::UpdateUnitStarted => "update_unit_started",
        }
    }
}

pub(crate) fn record(event: Event) {
    let Some(directory) = std::env::var_os("CHARIOX_RUNTIME_DIAGNOSTICS_DIR") else {
        return;
    };
    // Failure of optional observation must not change the runtime outcome.
    let _ = (|| -> io::Result<()> {
        let mut writer = WRITER
            .lock()
            .map_err(|_| io::Error::other("diagnostic lock"))?;
        if writer.is_none() {
            *writer = Some(create_journal(Path::new(&directory))?);
        }
        append(writer.as_mut().unwrap(), event)
    })();
}

#[cfg(unix)]
fn create_journal(directory: &Path) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    // Caller prepares a private directory. Never follow a directory symlink.
    let metadata = std::fs::symlink_metadata(directory)?;
    if !directory.is_absolute() || !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return Err(io::Error::other("diagnostic directory must be private"));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(format!("runtime-{}-{stamp}.jsonl", std::process::id())))?;
    if file.metadata()?.uid() != metadata.uid() {
        return Err(io::Error::other("diagnostic directory owner mismatch"));
    }
    File::open(directory)?.sync_all()?;
    Ok(file)
}

#[cfg(not(unix))]
fn create_journal(_: &Path) -> io::Result<File> {
    Err(io::Error::other("diagnostic journal requires Unix"))
}

fn append(file: &mut File, event: Event) -> io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format!(
        "{{\"schema\":1,\"atMs\":{now},\"pid\":{},\"event\":\"{}\"}}\n",
        std::process::id(),
        event.name()
    );
    if file.metadata()?.len() + line.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("diagnostic journal full"));
    }
    file.write_all(line.as_bytes())?;
    file.sync_data()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn mp07_mp11_durable_payload_free_journal_and_unsafe_paths() {
        let root = std::env::temp_dir().join(format!("path1-diagnostics-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut file = create_journal(&root).unwrap();
        append(&mut file, Event::PromptDispatch).unwrap();
        append(&mut file, Event::HeartbeatSent).unwrap();
        append(&mut file, Event::UpdateDownload).unwrap();
        let path = std::fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.contains("prompt_dispatch"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        file.set_len(MAX_BYTES).unwrap();
        assert!(append(&mut file, Event::HeartbeatFailed).is_err());
        let link = root.with_extension("link");
        symlink(&root, &link).unwrap();
        assert!(create_journal(&link).is_err());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(create_journal(&root).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
