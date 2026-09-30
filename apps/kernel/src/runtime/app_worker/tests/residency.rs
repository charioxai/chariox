//! Owner decision 6: only a wake armed during a tool call or an inbound event
//! counts as use. The wake pump stops a worker once `idle_ms` reaches its idle
//! period, after the pass's deliveries.
use crate::runtime::app_worker::Residency;

/// The wake pump's idle period (`IDLE_AFTER_MS`).
const IDLE: u64 = 10 * 60_000;
const MINUTE: u64 = 60_000;

#[test]
fn a_self_rearming_wake_loop_does_not_keep_the_app_resident_past_its_idle_deadline() {
    let worker = Residency::new(0);
    // A tool call arms the first wake; its delivery a minute later is use.
    worker.touch(0);
    worker.touch(MINUTE);
    // From then on the wake handler re-arms itself every minute.
    let deadline = MINUTE + IDLE;
    for now in (2..=10).map(|n| n * MINUTE) {
        worker.self_woken();
        assert_eq!(worker.idle_ms(now), now - MINUTE, "still within its use");
    }
    assert!(worker.idle_ms(deadline - 1) < IDLE);
    worker.self_woken();
    assert!(
        worker.idle_ms(deadline) >= IDLE,
        "stops at its idle deadline"
    );
}

#[test]
fn a_worker_started_only_for_its_own_wake_stops_once_the_wake_is_delivered() {
    let started = 20 * MINUTE;
    let worker = Residency::new(started);
    // Starting it is not use: until something is delivered it waits as before.
    assert_eq!(worker.idle_ms(started + 2_000), 2_000);
    worker.self_woken();
    assert!(worker.idle_ms(started + 2_000) >= IDLE);
    // A tool call that arrives meanwhile keeps it running from then on.
    worker.touch(started + 3_000);
    assert_eq!(worker.idle_ms(started + 4_000), 1_000);
    worker.self_woken();
    assert_eq!(worker.idle_ms(started + 5_000), 2_000);
}

#[test]
fn a_tool_armed_wake_still_counts_as_use() {
    let worker = Residency::new(0);
    // Armed by a tool call at 0 and delivered at 9 minutes: use then.
    worker.touch(9 * MINUTE);
    assert_eq!(worker.idle_ms(15 * MINUTE), 6 * MINUTE);
    assert!(worker.idle_ms(9 * MINUTE + IDLE - 1) < IDLE);
    // Started on demand for it after an idle stop, it stays for the idle period.
    let restarted = Residency::new(30 * MINUTE);
    restarted.touch(30 * MINUTE + 2_000);
    assert!(restarted.idle_ms(35 * MINUTE) < IDLE);
}

#[test]
fn no_idle_stop_or_eviction_interrupts_a_wake_being_delivered() {
    // Eviction takes a worker idle for a minute; the idle stop one idle for IDLE.
    const EVICTABLE: u64 = 60_000;
    let started = 20 * MINUTE;
    let worker = Residency::new(started);
    // A first wake of its own was delivered: the worker may stop.
    worker.self_woken();
    assert!(worker.idle_ms(started + 2_000) >= IDLE);
    // A second one is blocked in its handler while another App needs a slot.
    let second = worker.delivering();
    assert!(worker.idle_ms(started + 30_000) < EVICTABLE);
    drop(second);
    worker.self_woken();
    assert!(worker.idle_ms(started + 31_000) >= IDLE);

    // Holding off does not move the deadline of a worker in use.
    let used = Residency::new(0);
    used.touch(0);
    let late = used.delivering();
    assert_eq!(used.idle_ms(IDLE - 1_000), 0);
    drop(late);
    assert_eq!(used.idle_ms(IDLE), IDLE);
}
