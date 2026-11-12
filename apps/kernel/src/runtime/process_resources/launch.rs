//! Admission of immutable binary launches, before a user program exists.
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{admission, pool::Pool, MIB};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchWait {
    pub reason: admission::WaitReason,
    pub need_bytes: u64,
    pub free_bytes: u64,
}

pub(super) struct Reservation {
    pub(super) memory: u64,
    /// Predicted writes not yet consumed by this domain.
    pub(super) writes: u64,
    pub(super) volume: u64,
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
    reserved_volume_written: u64,
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
                .record(
                    &self.key,
                    self.peak.clone(),
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
        let writes: u64 = samples
            .iter()
            .map(|sample| admission::written(sample.pid))
            .sum();
        self.peak.memory = self.peak.memory.max(memory);
        self.peak.writes = self.peak.writes.max(writes);
        let mut destinations =
            std::collections::BTreeMap::<u64, (admission::ReceivingVolume, u64)>::new();
        let mut counted = std::collections::BTreeSet::new();
        for member in self.admission.0.members() {
            let member = member.lock().unwrap_or_else(|e| e.into_inner());
            if member.unit != self.id {
                continue;
            }
            for (pid, outputs) in &member.disk_writes {
                for output in outputs {
                    if !counted.insert((*pid, output.id)) {
                        continue;
                    }
                    let Some(volume) = admission::ReceivingVolume::capture(output) else {
                        continue;
                    };
                    let destination = destinations.entry(output.id).or_insert((volume, 0));
                    destination.1 = destination.1.saturating_add(output.written);
                }
            }
        }
        if let Some((volume, written)) = destinations.values().max_by_key(|(_, written)| *written) {
            if *written > self.peak.volume_writes {
                self.peak.volume = Some(volume.clone());
                self.peak.volume_writes = *written;
            }
        }
        let mut launches = self
            .admission
            .0
            .launches
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(reservation) = launches.get_mut(&self.id) else {
            return;
        };
        let written = destinations
            .get(&reservation.volume)
            .map_or(0, |(_, written)| *written);
        let consumed = written.saturating_sub(self.reserved_volume_written);
        self.reserved_volume_written = self.reserved_volume_written.max(written);
        if consumed > 0 {
            reservation.writes = reservation.writes.saturating_sub(consumed);
            // A free-space read preceding this consumption cannot credit it.
            self.admission
                .0
                .launch_generation
                .fetch_add(1, Ordering::AcqRel);
        }
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
        let protection = pool.protection();
        let seen = admission::seen_argv(0, argv, Some(cwd.to_path_buf()));
        loop {
            let generation = pool.launch_generation.load(Ordering::Acquire);
            let members = pool.members();
            let room = pool.room(&members);
            // Checking and inserting reservations is one transaction across binary
            // callers and observed-command admission.
            let ledger = pool.ledger.lock().unwrap_or_else(|e| e.into_inner());
            let expected = ledger.peaks.get(&seen.key).unwrap_or_default();
            let memory = expected.memory.max(16 * MIB);
            let mut launches = pool.launches.lock().unwrap_or_else(|e| e.into_inner());
            let light = admission::light(pool.ceiling);
            let launch_memory = room.remaining(&launches);
            let reserved = ledger.reserved_memory(&members, light, &launch_memory);
            let disk_reserved = ledger.reserved_disk(&members, light, &launches, &*pool.volume);
            // Consumption is observed before free space. A completion or write
            // observation since the memory sample requires a fresh snapshot.
            let disk = (expected.exact == seen.exact)
                .then(|| expected.volume.as_ref()?.disk(&*pool.volume))
                .flatten();
            let expected_writes = disk.map_or(0, |_| expected.volume_writes);
            if generation != pool.launch_generation.load(Ordering::Acquire) {
                continue;
            }
            let free = room
                .memory
                .saturating_sub(reserved)
                .saturating_sub(launch_memory.values().copied().fold(0, u64::saturating_add));
            if !force && protection.memory && pool.memory_emergency() && memory > free {
                return Err(LaunchWait {
                    reason: admission::WaitReason::Memory,
                    need_bytes: memory,
                    free_bytes: free,
                });
            }
            if let Some(disk) = disk {
                let free = disk.above_floor().saturating_sub(
                    launches
                        .values()
                        .filter(|r| r.volume == disk.id)
                        .fold(0u64, |sum, r| sum.saturating_add(r.writes))
                        .saturating_add(disk_reserved.get(&disk.id).copied().unwrap_or(0)),
                );
                if !force && protection.disk && disk.below_floor() && expected_writes > free {
                    return Err(LaunchWait {
                        reason: admission::WaitReason::Disk,
                        need_bytes: expected_writes,
                        free_bytes: free,
                    });
                }
            }
            launches.insert(
                id.to_string(),
                Reservation {
                    memory,
                    writes: expected_writes,
                    volume: disk.map_or(0, |d| d.id),
                },
            );
            pool.launch_generation.fetch_add(1, Ordering::AcqRel);
            return Ok(LaunchPermit {
                admission: self.clone(),
                id: id.to_string(),
                key: seen.key,
                peak: admission::Peak {
                    exact: seen.exact,
                    ..Default::default()
                },
                observed: false,
                reserved_volume_written: 0,
            });
        }
    }
}
