//! Kernel performance budgets (perf-budgets/kernel.json) that kernel tests check.
//!
//! `kernel.event_envelope_bytes` (perf-budgets/kernel.json): a state event a subscriber receives stays under the
//! budget, so one Room's change never costs every client a large read. Every kernel test that sends an event
//! through a local or relay subscription checks it and fails over the budget; release builds keep the relay
//! writer's large-frame log. Terminal output and kernel browser frames are byte streams with their own flow
//! control, not state, and are not counted.

const STREAMS: [&str; 2] = ["terminal_output", "kernel_browser_frame"];

/// The serialized size of `event` as it goes to a subscriber.
pub(crate) fn check_event(event: &'static str, bytes: usize) {
    if STREAMS.contains(&event) {
        return;
    }
    #[cfg(test)]
    assert!(
        bytes <= max_bytes(),
        "the {event} event is {bytes} bytes, over the kernel.event_envelope_bytes budget of {} \
         (perf-budgets/kernel.json): send what changed, not the whole state",
        max_bytes()
    );
    #[cfg(not(test))]
    let _ = bytes;
}

#[cfg(test)]
pub(crate) fn budget(name: &str) -> usize {
    let budgets: serde_json::Value =
        serde_json::from_str(include_str!("../../../../perf-budgets/kernel.json"))
            .expect("perf-budgets/kernel.json is JSON");
    budgets[name]["max"]
        .as_u64()
        .unwrap_or_else(|| panic!("perf-budgets/kernel.json has no {name}.max")) as usize
}

#[cfg(test)]
fn max_bytes() -> usize {
    static MAX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| budget("event_envelope_bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_event_over_the_budget_fails_the_test() {
        let over = max_bytes() + 1;
        let failed = std::panic::catch_unwind(|| check_event("session_snapshot", over))
            .expect_err("an oversized state event fails");
        let message = failed.downcast_ref::<String>().unwrap();
        assert!(message.contains("kernel.event_envelope_bytes"), "{message}");
        check_event("session_snapshot", max_bytes());
        check_event("terminal_output", over);
    }
}
