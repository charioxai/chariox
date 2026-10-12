//! MP-08 / MP-10: readiness hints wake the shared pump without moving provider
//! I/O or prompt settlement out of the provider actor.
use crate::runtime::state::RuntimeChangeSignal;
use std::collections::BTreeSet;
use std::sync::{mpsc, Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderOutputReadySignal {
    changes: Arc<RuntimeChangeSignal>,
    ready: Arc<Mutex<BTreeSet<String>>>,
}
impl ProviderOutputReadySignal {
    pub(crate) fn sequence(&self) -> u64 {
        self.changes.sequence()
    }
    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        self.changes.wait_for_change_after(sequence).await;
    }
    pub(crate) fn take_ready_provider_run_ids(&self) -> BTreeSet<String> {
        std::mem::take(
            &mut *self
                .ready
                .lock()
                .expect("provider output readiness poisoned"),
        )
    }
    fn record(&self, run_id: &str) {
        if self
            .ready
            .lock()
            .expect("provider output readiness poisoned")
            .insert(run_id.to_string())
        {
            self.changes.record_change();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderEventNotifier {
    binding: Arc<Mutex<Option<(String, ProviderOutputReadySignal)>>>,
}
impl ProviderEventNotifier {
    pub(crate) fn bind(&self, run_id: String, signal: ProviderOutputReadySignal) {
        *self
            .binding
            .lock()
            .expect("provider event notifier poisoned") = Some((run_id, signal));
        // Include anything queued before the runtime was installed.
        self.notify();
    }
    pub(crate) fn inherit_binding(&self, other: &Self) {
        *self
            .binding
            .lock()
            .expect("provider event notifier poisoned") = other
            .binding
            .lock()
            .expect("provider event notifier poisoned")
            .clone();
        self.notify();
    }
    pub(crate) fn notify(&self) {
        if let Some((run_id, signal)) = self
            .binding
            .lock()
            .expect("provider event notifier poisoned")
            .as_ref()
        {
            signal.record(run_id);
        }
    }
    pub(crate) fn send<T>(&self, tx: &mpsc::Sender<T>, value: T) -> Result<(), mpsc::SendError<T>> {
        tx.send(value)?;
        self.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queued_provider_events_wake_the_pump_and_coalesce_run_ids() {
        let signal = ProviderOutputReadySignal::default();
        let notifier = ProviderEventNotifier::default();
        notifier.bind("run-1".into(), signal.clone());
        signal.take_ready_provider_run_ids();
        let sequence = signal.sequence();
        let (tx, rx) = mpsc::channel();
        notifier.send(&tx, "first").unwrap();
        notifier.send(&tx, "second").unwrap();
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            signal.wait_for_change_after(sequence),
        )
        .await
        .unwrap();
        assert_eq!(
            signal.take_ready_provider_run_ids(),
            BTreeSet::from(["run-1".into()])
        );
        assert_eq!(rx.try_recv().unwrap(), "first");
        assert_eq!(rx.try_recv().unwrap(), "second");
        assert!(signal.take_ready_provider_run_ids().is_empty());
    }
    #[test]
    fn reconnect_inherits_the_binding_without_changing_the_reader_notifier() {
        let signal = ProviderOutputReadySignal::default();
        let old = ProviderEventNotifier::default();
        old.bind("run".into(), signal.clone());
        let next = ProviderEventNotifier::default();
        let reader = next.clone();
        next.inherit_binding(&old);
        signal.take_ready_provider_run_ids();
        reader.notify();
        assert_eq!(
            signal.take_ready_provider_run_ids(),
            BTreeSet::from(["run".into()])
        );
    }
}
