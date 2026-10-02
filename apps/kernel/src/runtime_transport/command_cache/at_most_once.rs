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
    pub(crate) async fn has_reserved(&self, command_id: &str) -> bool {
        self.results.lock().await.contains_key(command_id)
    }

    /// No result is visible until its final receipt has been appended and synced.
    pub(crate) async fn complete_at_most_once(
        &self,
        command_id: String,
        fingerprint: CommandFingerprint,
        response: Value,
        interrupted_response: Value,
    ) -> io::Result<()> {
        let mut cached = CachedCommandResult {
            response: serialized_response(&Some(response)),
            error: None,
            fingerprint,
            completed_at_ms: crate::session::unix_epoch_ms(),
        };
        let record = PersistentCommandResult {
            command_id: command_id.clone(),
            completed_at_ms: cached.completed_at_ms,
            result: cached.clone(),
        };
        let settled = match persistent_result_jsonl_bytes(&record) {
            Err(error) => Err(error),
            Ok(bytes) if bytes > COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES => {
                Err(io::Error::other("oversized at-most-once settlement"))
            }
            Ok(_) => {
                self.persist_completed_result(command_id.clone(), cached.clone())
                    .await
            }
        };
        if settled.is_err() {
            // Acceptance remains durable. Never expose an uncommitted success,
            // and never redispatch the effect after an ambiguous settlement.
            cached.response = serialized_response(&Some(interrupted_response));
        }
        self.publish_completed_result(&command_id, &cached).await;
        self.apply_retention_to_completed_results(
            &command_id,
            cached_command_result_memory_bytes(&command_id, &cached),
        )
        .await;
        settled
    }

    pub(crate) async fn reserve_at_most_once(
        &self,
        command_id: &str,
        fingerprint: &CommandFingerprint,
        interrupted_response: Value,
    ) -> io::Result<CommandReservation> {
        if !self.retention.at_most_once {
            return Err(io::Error::other(
                "at-most-once reservation requires non-evicting persistence",
            ));
        }
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
            response: serialized_response(&Some(interrupted_response)),
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
