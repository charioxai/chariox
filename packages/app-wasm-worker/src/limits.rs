use serde::{Deserialize, Serialize};
use wasmtime::ResourceLimiter;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub memory_bytes: usize,
    pub slice_ms: u64,
    pub call_ms: u64,
    pub cancel_grace_ms: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 << 20,
            slice_ms: 1_000,
            call_ms: 30_000,
            cancel_grace_ms: 250,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> wasmtime::Result<()> {
        wasmtime::ensure!(
            (1 << 20..=1_usize << 32).contains(&self.memory_bytes),
            "memory limit must be between 1 MiB and 4 GiB"
        );
        wasmtime::ensure!(
            (1..=30_000).contains(&self.slice_ms),
            "slice limit must be 1..30000 ms"
        );
        wasmtime::ensure!(
            (1..=30_000).contains(&self.call_ms),
            "call limit must be 1..30000 ms"
        );
        wasmtime::ensure!(
            (1..=1_000).contains(&self.cancel_grace_ms),
            "cancel grace must be 1..1000 ms"
        );
        Ok(())
    }
}
#[derive(Debug)]
pub struct MemoryLimit;
impl std::fmt::Display for MemoryLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("App instance memory limit exceeded")
    }
}
impl std::error::Error for MemoryLimit {}

pub struct Memory {
    limit: usize,
    used: usize,
    last_growth: usize,
    tables: usize,
    last_table_growth: usize,
}
impl Memory {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            last_growth: 0,
            tables: 0,
            last_table_growth: 0,
        }
    }
}
impl ResourceLimiter for Memory {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let growth = desired.saturating_sub(current);
        if growth > self.limit.saturating_sub(self.used) {
            return Err(MemoryLimit.into());
        }
        self.used += growth;
        self.last_growth = growth;
        Ok(true)
    }
    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        self.used -= self.last_growth;
        self.last_growth = 0;
        Ok(())
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let growth = desired.saturating_sub(current);
        if growth > (1_usize << 20).saturating_sub(self.tables) {
            return Err(MemoryLimit.into());
        }
        self.tables += growth;
        self.last_table_growth = growth;
        Ok(true)
    }
    fn table_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        self.tables -= self.last_table_growth;
        self.last_table_growth = 0;
        Ok(())
    }
    fn instances(&self) -> usize {
        32
    }
    fn memories(&self) -> usize {
        8
    }
    fn tables(&self) -> usize {
        32
    }
}

#[derive(Debug)]
pub(crate) struct Stop(pub(crate) &'static str);
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Stop {}

/// Epochs charge only time in the current guest task, excluding broker waits.
pub(crate) fn invocation_limits(store: &mut wasmtime::Store<crate::host::Host>, limits: Limits) {
    use std::time::Duration;
    use wasmtime::UpdateDeadline;
    let slice = Duration::from_millis(limits.slice_ms);
    store.epoch_deadline_callback(move |store| {
        let attribution = &store.data().attribution;
        if let Some(scope) = attribution.current() {
            if scope.reason().is_none() && tokio::time::Instant::now() >= scope.deadline() {
                scope.cancel("DEADLINE_EXCEEDED");
            }
            // A resumed guest may first need to process cancellation and free
            // frames. Epoch ticks elapsed during a broker wait do not revoke
            // that grace; the first cancellation owns its fixed start time.
            if let Some(reason) =
                scope.expired_cancellation(Duration::from_millis(limits.cancel_grace_ms))
            {
                return Err(Stop(reason).into());
            }
        }
        if attribution
            .slice_started()
            .is_some_and(|started| started.elapsed() >= slice)
        {
            return Err(Stop("APP_CPU_SLICE").into());
        }
        Ok(UpdateDeadline::Yield(1))
    });
    store.set_epoch_deadline(1);
}
