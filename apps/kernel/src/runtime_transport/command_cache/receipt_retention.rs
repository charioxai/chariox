//! Payload receipts are bounded; durable identity markers prevent redispatch forever.
use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub(super) struct ReceiptRetention {
    // Compact fixed-size digests have a separate bound: at most 50 MiB / 65 entries.
    expired: BTreeSet<[u8; 32]>,
    append_unavailable: bool,
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
        let mut reader = io::BufReader::new(&file);
        let mut offset = 0;
        loop {
            let mut line = Vec::new();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if line.last() != Some(&b'\n') {
                // Payload removal requires a complete, synced marker. A torn
                // final append therefore still has its durable payload receipt.
                file.set_len(offset)?;
                break;
            }
            if line.len() != 65
                || !line[..64]
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            {
                return Err(io::Error::other("corrupt expired receipt marker"));
            }
            let digest = std::array::from_fn(|index| {
                u8::from_str_radix(
                    std::str::from_utf8(&line[index * 2..index * 2 + 2]).unwrap(),
                    16,
                )
                .unwrap()
            });
            expired.insert(digest);
            offset += line.len() as u64;
        }
        // A process crash may leave readable marker bytes which were never
        // synced. Make them durable before recovery removes any payload receipt.
        sync(&file, &path)?;
        Ok(Self {
            expired,
            append_unavailable: false,
        })
    }

    pub(super) fn marker_path(path: &PathBuf) -> PathBuf {
        path.with_extension("expired")
    }

    fn digest(command_id: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        Sha256::digest(command_id.as_bytes()).into()
    }

    fn identity(command_id: &str) -> String {
        Self::digest(command_id)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(super) fn contains(&self, command_id: &str) -> bool {
        self.expired.contains(&Self::digest(command_id))
    }

    pub(super) fn expire(&mut self, path: &PathBuf, command_id: &str) -> io::Result<()> {
        self.expire_with_write(path, command_id, |file, bytes| file.write_all(bytes))
    }

    fn expire_with_write(
        &mut self,
        path: &PathBuf,
        command_id: &str,
        write: impl FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.append_unavailable {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                at_most_once::ReceiptCapacityError,
            ));
        }
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
        let before = file.metadata()?.len();
        if before.saturating_add(65) > COMMAND_RESULT_CACHE_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                at_most_once::ReceiptCapacityError,
            ));
        }
        // An ambiguous write must fail closed in this process too.
        let digest = Self::digest(command_id);
        let inserted = self.expired.insert(digest);
        if let Err(error) = write(&mut file, format!("{marker}\n").as_bytes()) {
            // Restore the append boundary before allowing another eviction. If
            // that fails, wait for startup's torn-tail recovery, failing closed.
            if file.set_len(before).and_then(|()| file.sync_all()).is_err() {
                self.append_unavailable = true;
            } else if inserted {
                // The marker is definitely absent and its response is intact.
                // Do not let a later, different eviction filter that response.
                self.expired.remove(&digest);
            }
            return Err(error);
        }
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
