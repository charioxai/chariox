//! Reuse the command-result journal for durable, non-evicting reservations.
use super::*;

impl CommandFingerprint {
    pub(crate) fn for_app_control(command: &KernelCommand, request: &LocalDaemonRequest) -> Self {
        use sha2::{Digest, Sha256};
        let mut fingerprint = Self::from_command_and_request(command, request);
        // Bind receipts to authenticated caller and exact input, with a strong digest.
        fingerprint.source = serde_json::json!({
            "source": command.source,
            "caller": command.caller,
            "request": format!("{:x}", Sha256::digest(serde_json::to_vec(request).unwrap())),
        })
        .to_string();
        fingerprint
    }
}

impl CommandResultCache {
    pub(crate) async fn reserve_at_most_once(
        &self,
        command_id: &str,
        fingerprint: &CommandFingerprint,
        interrupted_response: Value,
    ) -> io::Result<CommandReservation> {
        let mut results = self.results.lock().await;
        match results.get_mut(command_id) {
            Some(CommandResultEntry::Completed(cached)) => {
                if cached.fingerprint != *fingerprint {
                    return Ok(CommandReservation::Conflict);
                }
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(cached.clone());
                return Ok(CommandReservation::Wait(rx));
            }
            Some(CommandResultEntry::Pending {
                fingerprint: prior,
                waiters,
            }) => {
                if prior != fingerprint {
                    return Ok(CommandReservation::Conflict);
                }
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                return Ok(CommandReservation::Wait(rx));
            }
            None => {}
        }
        if results.len() >= self.retention.max_entries {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "at-most-once receipt capacity reached",
            ));
        }
        let persistence = self
            .persistence
            .as_ref()
            .ok_or_else(|| io::Error::other("at-most-once receipts require persistence"))?;
        let cached = CachedCommandResult {
            response: Box::new(Some(interrupted_response)),
            error: None,
            completed_at_ms: crate::session::unix_epoch_ms(),
            fingerprint: fingerprint.clone(),
        };
        // Even an ambiguous write failure must block redispatch in this process.
        results.insert(
            command_id.into(),
            CommandResultEntry::Completed(cached.clone()),
        );
        let _guard = persistence.io_lock.lock().await;
        if let Some(parent) = persistence.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = PersistentCommandResult {
            command_id: command_id.into(),
            completed_at_ms: cached.completed_at_ms,
            result: cached,
        };
        append_persistent_result(&persistence.path, &record)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&persistence.path)?
            .sync_all()?;
        if let Some(parent) = persistence.path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        results.insert(
            command_id.into(),
            CommandResultEntry::Pending {
                fingerprint: fingerprint.clone(),
                waiters: Vec::new(),
            },
        );
        Ok(CommandReservation::Dispatch)
    }
}
