//! Admission of immutable binary launches, before a user program exists.
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{admission, pool::Pool, MIB};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchWait {
    pub reason: admission::WaitReason,
    pub need_bytes: u64,
    pub free_bytes: u64,
}

pub(super) struct Reservation {
    pub(super) memory: u64,
    writes: u64,
    volume: u64,
}

#[derive(Clone)]
pub(crate) struct LaunchAdmission(Arc<Pool>);

impl Default for LaunchAdmission {
    fn default() -> Self {
        Self(Pool::shared())
    }
}

pub(crate) struct LaunchPermit {
    admission: LaunchAdmission,
    id: String,
    key: String,
    peak: admission::Peak,
    observed: bool,
}

impl Drop for LaunchPermit {
    fn drop(&mut self) {
        {
            let mut launches = self
                .admission
                .0
                .launches
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            launches.remove(&self.id);
            self.admission
                .0
                .launch_generation
                .fetch_add(1, Ordering::AcqRel);
        }
        if self.observed {
            self.admission
                .0
                .ledger
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .peaks
                .remember(
                    &self.key,
                    self.peak,
                    admission::light(self.admission.0.ceiling),
                );
        }
    }
}

impl LaunchPermit {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn prepare(&self) -> std::io::Result<super::PreparedDomain> {
        let cap = super::configured_cap(
            std::env::var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB")
                .ok()
                .as_deref(),
        )?;
        Ok(super::PreparedDomain::in_pool(
            &self.id,
            cap,
            self.admission.0.clone(),
        ))
    }

    pub(crate) fn observe(&mut self) {
        let samples = super::samples_for_key(&self.id);
        self.observed |= !samples.is_empty();
        let memory = samples.iter().map(|sample| sample.resident_set_bytes).sum();
        let writes = samples
            .iter()
            .map(|sample| admission::written(sample.pid))
            .sum();
        self.peak.memory = self.peak.memory.max(memory);
        self.peak.writes = self.peak.writes.max(writes);
    }
}

impl LaunchAdmission {
    #[cfg(test)]
    pub(crate) fn limited(memory: u64, free_disk: u64) -> Self {
        Self(Arc::new(Pool::with_volume(
            memory,
            Box::new(move |_| {
                Some(admission::Space {
                    id: 1,
                    free: free_disk,
                    total: free_disk.max(1),
                })
            }),
        )))
    }

    pub(crate) fn reserve(
        &self,
        id: &str,
        argv: &[String],
        cwd: &Path,
        force: bool,
    ) -> Result<LaunchPermit, LaunchWait> {
        let pool = &self.0;
        let seen = admission::seen_argv(0, argv, Some(cwd.to_path_buf()));
        loop {
            let generation = pool.launch_generation.load(Ordering::Acquire);
            let members = pool.members();
            let (room, _) = pool.room(&members);
            let disk = (pool.volume)(cwd);
            // Observations and filesystem reads happen outside this lock. The reservation
            // check and insertion are one transaction across all binary callers.
            let ledger = pool.ledger.lock().unwrap_or_else(|e| e.into_inner());
            let expected = ledger.peaks.get(&seen.key).unwrap_or_default();
            let memory = expected.memory.max(16 * MIB);
            let reserved = ledger.reserved_memory(&members, admission::light(pool.ceiling));
            let mut launches = pool.launches.lock().unwrap_or_else(|e| e.into_inner());
            // A completion can consume reserved disk space. Do not use a free-space
            // sample taken before that reservation disappeared.
            if generation != pool.launch_generation.load(Ordering::Acquire) {
                continue;
            }
            let free = room.saturating_sub(reserved).saturating_sub(
                launches
                    .values()
                    .fold(0u64, |sum, r| sum.saturating_add(r.memory)),
            );
            if !force && memory > free {
                return Err(LaunchWait {
                    reason: admission::WaitReason::Memory,
                    need_bytes: memory,
                    free_bytes: free,
                });
            }
            if let Some(disk) = disk {
                let floor = (disk.total / 20).max(10 * 1024 * MIB);
                let free = disk.free.saturating_sub(floor).saturating_sub(
                    launches
                        .values()
                        .filter(|r| r.volume == disk.id)
                        .fold(0u64, |sum, r| sum.saturating_add(r.writes)),
                );
                if !force && (disk.free <= floor || expected.writes > free) {
                    return Err(LaunchWait {
                        reason: admission::WaitReason::Disk,
                        need_bytes: expected.writes.max(1),
                        free_bytes: free,
                    });
                }
            }
            launches.insert(
                id.to_string(),
                Reservation {
                    memory,
                    writes: expected.writes,
                    volume: disk.map_or(0, |d| d.id),
                },
            );
            pool.launch_generation.fetch_add(1, Ordering::AcqRel);
            return Ok(LaunchPermit {
                admission: self.clone(),
                id: id.to_string(),
                key: seen.key,
                peak: admission::Peak::default(),
                observed: false,
            });
        }
    }
}
