//! MP-08 / MP-09 / MP-10 / MP-11: one lifecycle sweep runs at a time. A sweep reads the whole task
//! ledger, and every settled turn, prompt dispatch and Room change asks for one, so asking while a sweep
//! runs only marks that it must run once more instead of starting another. Peer outcome delivery
//! passes run one at a time the same way, apart from the sweep.
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Running,
    RunningAskedAgain,
}

#[derive(Clone, Default)]
pub(super) struct SweepFlight {
    phase: Arc<Mutex<Phase>>,
    #[cfg(test)]
    runs: Arc<std::sync::atomic::AtomicUsize>,
}

impl SweepFlight {
    fn phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the caller must run the sweep; otherwise the running one runs again.
    pub(super) fn ask(&self) -> Option<SweepRun> {
        let mut phase = self.phase();
        if *phase == Phase::Idle {
            *phase = Phase::Running;
            return Some(SweepRun {
                flight: self.clone(),
                finished: false,
            });
        }
        *phase = Phase::RunningAskedAgain;
        None
    }

    #[cfg(test)]
    pub(super) fn record_run(&self) {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn runs(&self) -> usize {
        self.runs.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        *self.phase() == Phase::Idle
    }
}

/// The right to run sweeps, held until a sweep ends with nobody having asked again. Dropping it
/// early, even by a panic, lets the next ask run one.
pub(super) struct SweepRun {
    flight: SweepFlight,
    finished: bool,
}

impl SweepRun {
    /// Whether a sweep was asked for while the last one ran; if not, the right is given up.
    pub(super) fn asked_again(&mut self) -> bool {
        let mut phase = self.flight.phase();
        if *phase == Phase::RunningAskedAgain {
            *phase = Phase::Running;
            return true;
        }
        *phase = Phase::Idle;
        self.finished = true;
        false
    }
}

impl Drop for SweepRun {
    fn drop(&mut self) {
        if !self.finished {
            *self.flight.phase() = Phase::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_burst_of_sweep_requests_runs_the_sweep_at_most_twice() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let app = DaemonApp::bootstrap(config).unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )
        .runtime_state();
        // The test runtime runs spawned sweeps only once this task awaits, so all asks arrive first.
        for _ in 0..20 {
            runtime.schedule_agent_lifecycle_sweep();
        }
        let mut seen = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let runs = runtime.lifecycle_sweeps.runs();
            if runs == seen && runs > 0 && runtime.lifecycle_sweeps.is_idle() {
                break;
            }
            seen = runs;
        }
        let runs = runtime.lifecycle_sweeps.runs();
        assert!(
            (1..=2).contains(&runs),
            "20 requests made {runs} sweeps; one running sweep must absorb them"
        );
    }
}
