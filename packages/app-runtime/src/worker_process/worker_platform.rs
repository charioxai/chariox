//! Private worker monitoring policy; not an App-controlled runtime option.
//! These thresholds terminate an owned worker after observation. They are not
//! hard memory/CPU quotas and exclude separately admitted broker/browser work.

use super::WorkerError;
use std::time::Instant;

use super::monitor::RESOURCE_CHECK_INTERVAL as CHECK_INTERVAL;
const CPU_BURST_NS: u128 = 250_000_000;

fn cpu_nanoseconds(user: u64, system: u64, numer: u32, denom: u32) -> Result<u64, WorkerError> {
    if numer == 0 || denom == 0 {
        return Err(WorkerError::ResourceTelemetry);
    }
    let ticks = user as u128 + system as u128;
    u64::try_from(ticks * numer as u128 / denom as u128).map_err(|_| WorkerError::ResourceTelemetry)
}

#[derive(Clone, Copy)]
pub(super) struct ResourcePolicy {
    memory_bytes: u64,
    threads: u32,
}

impl ResourcePolicy {
    // Candidate Phase 1 worker policy. Actual pinned-Node/reference-App and
    // aggregate-pressure measurements must validate it before release.
    pub(super) const PHASE1: Self = Self {
        memory_bytes: 512 * 1024 * 1024,
        threads: 64,
    };
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    start_time: u64,
    physical_bytes: u64,
    resident_bytes: u64,
    threads: u32,
    cpu_ns: u64,
}

impl Sample {
    fn memory_bytes(self) -> u64 {
        self.physical_bytes.max(self.resident_bytes)
    }

    fn validate(self, policy: ResourcePolicy) -> Result<(), WorkerError> {
        if self.start_time == 0 || self.memory_bytes() == 0 || self.threads == 0 {
            return Err(WorkerError::ResourceTelemetry);
        }
        if self.memory_bytes() > policy.memory_bytes {
            return Err(WorkerError::MemoryLimit);
        }
        if self.threads > policy.threads {
            return Err(WorkerError::ThreadLimit);
        }
        Ok(())
    }
}

struct Budget {
    policy: ResourcePolicy,
    last: Sample,
    sampled: Instant,
    cpu_credit_ns: u128,
}

impl Budget {
    fn new(policy: ResourcePolicy, sample: Sample, now: Instant) -> Result<Self, WorkerError> {
        sample.validate(policy)?;
        Ok(Self {
            policy,
            last: sample,
            sampled: now,
            cpu_credit_ns: CPU_BURST_NS,
        })
    }

    fn observe(&mut self, sample: Sample, now: Instant) -> Result<(), WorkerError> {
        let elapsed = now
            .checked_duration_since(self.sampled)
            .ok_or(WorkerError::ResourceTelemetry)?;
        if sample.start_time != self.last.start_time {
            return Err(WorkerError::ResourceTelemetry);
        }
        let consumed = sample
            .cpu_ns
            .checked_sub(self.last.cpu_ns)
            .ok_or(WorkerError::ResourceTelemetry)? as u128;
        sample.validate(self.policy)?;
        // One CPU: one nanosecond of credit per elapsed nanosecond. Credit is
        // capped AFTER consumption so a full-rate worker does not lose its
        // burst merely because observations occur at discrete intervals.
        let available = self.cpu_credit_ns.saturating_add(elapsed.as_nanos());
        if consumed > available {
            return Err(WorkerError::CpuLimit);
        }
        self.cpu_credit_ns = (available - consumed).min(CPU_BURST_NS);
        self.last = sample;
        self.sampled = now;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(super) use macos::MacResourceMonitor;

#[cfg(test)]
mod tests;
