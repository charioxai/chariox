//! Reuse the command-result journal for durable reservations and guarded LRU retention.
use super::*;

#[derive(Debug, thiserror::Error)]
#[error("at-most-once receipt capacity reached")]
pub(super) struct ReceiptCapacityError;

/// OS ENOMEM is also OutOfMemory, but must never authorize a control effect.
pub(crate) fn is_receipt_capacity_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|error| error.is::<ReceiptCapacityError>())
}

#[derive(Debug, thiserror::Error)]
#[error("receipt expired")]
struct ReceiptExpiredError;

pub(crate) fn is_receipt_expired_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|error| error.is::<ReceiptExpiredError>())
}

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
        let retention = self.receipt_retention.lock().await;
        retention.contains(command_id) || self.results.lock().await.contains_key(command_id)
    }

    /// No result is visible until its final receipt has been appended and synced.
    pub(crate) async fn complete_at_most_once(
        &self,
        command_id: String,
        fingerprint: CommandFingerprint,
        response: Value,
        interrupted_response: Value,
    ) -> io::Result<()> {
        let _retention = self.receipt_retention.lock().await;
        let now = crate::session::unix_epoch_ms();
        let completed_at_ms = match self.results.lock().await.get(&command_id) {
            Some(CommandResultEntry::Pending { accepted_at_ms, .. }) => now.max(*accepted_at_ms),
            Some(CommandResultEntry::Completed(result)) => now.max(result.completed_at_ms),
            None => now,
        };
        let mut cached = CachedCommandResult {
            response: serialized_response(&Some(response)),
            error: None,
            fingerprint,
            completed_at_ms,
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
                "at-most-once reservation requires durable persistence",
            ));
        }
        let mut retention = self.receipt_retention.lock().await;
        if retention.contains(command_id) {
            return Err(io::Error::other(ReceiptExpiredError));
        }
        let mut results = self.results.lock().await;
        match results.get_mut(command_id) {
            Some(CommandResultEntry::Completed(cached)) => {
                if cached.fingerprint != *fingerprint {
                    return Ok(CommandReservation::Conflict);
                }
                let cached = cached.clone();
                drop(results);
                // Preserve the completion age but persist the access order for LRU
                // across restarts. Touching a receipt never extends its age window.
                self.persist_completed_result(command_id.into(), cached.clone())
                    .await?;
                let mut order = self.order.lock().await;
                order.retain(|id| id != command_id);
                order.push_back(command_id.into());
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(cached);
                return Ok(CommandReservation::Wait(rx));
            }
            Some(CommandResultEntry::Pending {
                fingerprint: prior,
                waiters,
                ..
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
            drop(results);
            self.evict_expired_receipt(&mut retention).await?;
            results = self.results.lock().await;
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
        {
            let mut order = self.order.lock().await;
            order.retain(|id| id != command_id);
            order.push_back(command_id.into());
        }
        let _guard = persistence.io_lock.lock().await;
        if let Some(parent) = persistence.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = PersistentCommandResult {
            command_id: command_id.into(),
            completed_at_ms: cached.completed_at_ms,
            result: cached.clone(),
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
                accepted_at_ms: cached.completed_at_ms,
                waiters: Vec::new(),
            },
        );
        Ok(CommandReservation::Dispatch)
    }
}
