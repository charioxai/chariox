//! Managed work shares one admission fence and reports its activity transitions.
//! Terminal commands, execs and Project Environment operations keep the machine
//! running until their tasks settle, and cannot start behind a reserved stop.

use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::error::DaemonError;
use crate::runtime::state::{ManagedKernelAdmissionGuard, ManagedKernelQuiescenceGate};

struct Link {
    gate: Arc<ManagedKernelQuiescenceGate>,
    /// Records the kernel's activity transition once running work changed.
    work_changed: Weak<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
pub(crate) struct ManagedWork {
    link: OnceLock<Link>,
    working: Mutex<bool>,
    reports: crate::blocking_io::Serial,
}

impl ManagedWork {
    pub(crate) fn link(
        &self,
        gate: Arc<ManagedKernelQuiescenceGate>,
        work_changed: Weak<dyn Fn() + Send + Sync>,
    ) {
        let _ = self.link.set(Link { gate, work_changed });
    }

    /// Hold the guard while a command is committed: a stop cannot be reserved
    /// in between, and none is admitted once it is. Ordinary kernels have no
    /// fence.
    pub(crate) fn admission(&self) -> Result<Option<ManagedKernelAdmissionGuard<'_>>, DaemonError> {
        self.link
            .get()
            .map(|link| link.gate.admission_guard())
            .transpose()
    }

    /// Tell the kernel when the answer to "is a command running" changed.
    /// Called without any work registry lock held.
    pub(crate) fn report(&self, is_working: impl FnOnce() -> bool) {
        let Some(link) = self.link.get() else {
            return;
        };
        // A late idle sample must not overwrite a newer busy report and lose its completion.
        let _report = self.reports.acquire();
        let mut working = self
            .working
            .lock()
            .expect("managed work report mutex poisoned");
        let is_working = is_working();
        if *working != is_working {
            *working = is_working;
            drop(working);
            if let Some(work_changed) = link.work_changed.upgrade() {
                work_changed();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn overlapping_reports_do_not_lose_the_final_idle_transition() {
        let scratch = crate::test_support::TestWorktree::new("managed-report-order");
        let store = crate::durable_state::DurableKernelStateStore::open(
            scratch.path().join("activity.sqlite"),
        )
        .unwrap();
        let gate = ManagedKernelQuiescenceGate::restore(
            store,
            "report-kernel".into(),
            Arc::new(std::sync::Mutex::new(())),
        )
        .unwrap();
        let actual = Arc::new(AtomicBool::new(true));
        let observations = Arc::new(std::sync::Mutex::new(Vec::new()));
        let callback: Arc<dyn Fn() + Send + Sync> = {
            let actual = actual.clone();
            let observations = observations.clone();
            Arc::new(move || {
                observations
                    .lock()
                    .unwrap()
                    .push(actual.load(Ordering::Acquire))
            })
        };
        let work = Arc::new(ManagedWork::default());
        work.link(gate, Arc::downgrade(&callback));
        work.report(|| true);

        let (sampled_tx, sampled_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        actual.store(false, Ordering::Release);
        let finishing = {
            let work = work.clone();
            std::thread::spawn(move || {
                work.report(|| {
                    sampled_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    false
                })
            })
        };
        sampled_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        actual.store(true, Ordering::Release);
        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (reported_tx, reported_rx) = mpsc::channel();
        let starting = {
            let work = work.clone();
            std::thread::spawn(move || {
                attempted_tx.send(()).unwrap();
                work.report(|| true);
                reported_tx.send(()).unwrap();
            })
        };
        attempted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // A newer report must wait for the paused sample and its callback to settle.
        let overtook = reported_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_tx.send(()).unwrap();
        finishing.join().unwrap();
        starting.join().unwrap();
        actual.store(false, Ordering::Release);
        work.report(|| false);
        let observations = observations.lock().unwrap();
        assert_eq!(
            observations.last(),
            Some(&false),
            "the last completion must report idle"
        );
        assert!(!overtook, "a newer report overtook a pending sample");
    }
}
