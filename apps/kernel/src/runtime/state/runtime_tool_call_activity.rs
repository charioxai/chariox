//! Tracks provider MCP requests until their HTTP handlers finish. Provider turn
//! notifications can precede the completion of those handlers, so prompt
//! settlement must not mistake a still-running tool for an idle agent.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::ProviderOutputDeadlineStore;

#[derive(Clone, Default)]
pub(super) struct RuntimeToolCallActivity {
    counts: Arc<Mutex<BTreeMap<String, usize>>>,
}

pub(super) struct RuntimeToolCallGuard {
    activity: RuntimeToolCallActivity,
    provider_run_ids: Vec<String>,
    output_deadlines: ProviderOutputDeadlineStore,
}

impl RuntimeToolCallActivity {
    pub(super) fn begin(
        &self,
        provider_run_ids: impl IntoIterator<Item = String>,
        output_deadlines: ProviderOutputDeadlineStore,
    ) -> RuntimeToolCallGuard {
        let provider_run_ids = provider_run_ids.into_iter().collect::<Vec<_>>();
        let mut counts = self.counts.lock().expect("runtime tool activity poisoned");
        for provider_run_id in &provider_run_ids {
            *counts.entry(provider_run_id.clone()).or_default() += 1;
        }
        RuntimeToolCallGuard {
            activity: self.clone(),
            provider_run_ids,
            output_deadlines,
        }
    }

    pub(super) fn active_count(&self, provider_run_id: &str) -> usize {
        self.counts
            .lock()
            .expect("runtime tool activity poisoned")
            .get(provider_run_id)
            .copied()
            .unwrap_or_default()
    }
}

impl Drop for RuntimeToolCallGuard {
    fn drop(&mut self) {
        let mut counts = self
            .activity
            .counts
            .lock()
            .expect("runtime tool activity poisoned");
        for provider_run_id in &self.provider_run_ids {
            let Some(count) = counts.get_mut(provider_run_id) else {
                continue;
            };
            *count -= 1;
            if *count == 0 {
                counts.remove(provider_run_id);
                self.output_deadlines
                    .schedule(provider_run_id, crate::session::unix_epoch_ms());
            }
        }
    }
}
