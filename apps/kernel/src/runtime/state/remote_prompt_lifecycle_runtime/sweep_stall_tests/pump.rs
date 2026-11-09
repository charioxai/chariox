//! MP-08 / MP-09 / MP-10 / MP-11: the one maintenance pump does not wait for the lifecycle sweep.
use super::*;

/// The sweep waits up to `PROBE_TIMEOUT` (5 s) for a worker that does not answer pings.
const PUMP_BOUND: std::time::Duration = std::time::Duration::from_secs(4);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_transport_pump_does_not_wait_for_a_sweep_held_by_a_silent_worker() {
    let f = Stalled::new(false).await;
    tokio::time::timeout(PUMP_BOUND, f.runtime.pump_transport_runtime())
        .await
        .expect(
            "the pump waits for the lifecycle sweep, so every other maintenance step waits too",
        );
}
