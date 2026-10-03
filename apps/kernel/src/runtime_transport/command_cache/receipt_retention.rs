//! Payload receipts are bounded; durable identity markers prevent redispatch forever.
use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub(super) struct ReceiptRetention {
    expired: BTreeSet<String>,
}

impl ReceiptRetention {
    pub(super) fn load(path: &PathBuf) -> io::Result<Self> {
        Self::load_with_sync(path, |file, path| {
            file.sync_all()?;
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })
    }

    fn load_with_sync(
        path: &PathBuf,
        sync: impl FnOnce(&fs::File, &PathBuf) -> io::Result<()>,
    ) -> io::Result<Self> {
        let path = Self::marker_path(path);
        let file = match fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() > COMMAND_RESULT_CACHE_MAX_BYTES {
            return Err(io::Error::other(
                "expired receipt markers exceed the load limit",
            ));
        }
        let mut expired = BTreeSet::new();
        for line in io::BufReader::new(&file).lines() {
            let line = line?;
            if line.len() != 64
                || !line
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(io::Error::other("corrupt expired receipt marker"));
            }
            expired.insert(line);
        }
        // A process crash may leave readable marker bytes which were never
        // synced. Make them durable before recovery removes any payload receipt.
        sync(&file, &path)?;
        Ok(Self { expired })
    }

    pub(super) fn marker_path(path: &PathBuf) -> PathBuf {
        path.with_extension("expired")
    }

    fn identity(command_id: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(command_id.as_bytes()))
    }

    pub(super) fn contains(&self, command_id: &str) -> bool {
        self.expired.contains(&Self::identity(command_id))
    }

    pub(super) fn expire(&mut self, path: &PathBuf, command_id: &str) -> io::Result<()> {
        let marker = Self::identity(command_id);
        let path = Self::marker_path(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // Refuse new work rather than discard a replay fence at the storage bound.
        if file.metadata()?.len().saturating_add(65) > COMMAND_RESULT_CACHE_MAX_BYTES {
            return Err(io::Error::other("expired receipt marker capacity reached"));
        }
        // An ambiguous write must fail closed in this process too.
        self.expired.insert(marker.clone());
        writeln!(file, "{marker}")?;
        file.sync_all()?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

impl CommandResultCache {
    /// Called with receipt_retention held, serializing acceptance and settlement.
    pub(super) async fn evict_expired_receipt(
        &self,
        retention: &mut ReceiptRetention,
    ) -> io::Result<()> {
        let mut order = self.order.lock().await;
        let mut results = self.results.lock().await;
        let now = crate::session::unix_epoch_ms();
        let candidate = order
            .iter()
            .find(|id| match results.get(*id) {
                Some(CommandResultEntry::Completed(result)) => {
                    result.completed_at_ms != 0
                        && now.saturating_sub(result.completed_at_ms) > APP_RECEIPT_RETENTION_MS
                }
                _ => false, // Pending effects and unknown-age legacy receipts are protected.
            })
            .cloned();
        let Some(id) = candidate else {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                at_most_once::ReceiptCapacityError,
            ));
        };
        let persistence = self.persistence.as_ref().unwrap();
        let _guard = persistence.io_lock.lock().await;
        // Fence the identity durably BEFORE dropping its response. A crash between
        // the two writes leaves a redundant old receipt, which recovery filters.
        retention.expire(&persistence.path, &id)?;
        let loaded = read_persistent_results(&persistence.path, self.retention)?;
        let live = loaded
            .entries
            .into_iter()
            .map(|entry| entry.entry)
            .filter(|entry| !retention.contains(&entry.command_id))
            .collect::<Vec<_>>();
        rewrite_persistent_results(&persistence.path, &live)?;
        results.remove(&id);
        order.retain(|entry| entry != &id);
        let mut memory = self.memory_accounting.lock().await;
        if let Some(bytes) = memory.by_command_id.remove(&id) {
            memory.total_estimated_bytes = memory.total_estimated_bytes.saturating_sub(bytes);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
