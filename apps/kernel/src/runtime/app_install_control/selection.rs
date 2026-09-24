//! Bounded round-robin admission, independent of kernel tick frequency.
use super::*;

pub(super) fn ready(state: &mut State, now: Instant, limit: usize) -> Vec<Key> {
    let mut eligible = state
        .entries
        .iter()
        .filter(|(_, entry)| {
            !entry.busy
                && now >= entry.next
                && !matches!(entry.step, Step::Waiting { .. } | Step::Done)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(cursor) = &state.job_cursor {
        let split = eligible.partition_point(|key| key <= cursor);
        eligible.rotate_left(split);
    }
    eligible.truncate(limit);
    if let Some(last) = eligible.last() {
        state.job_cursor = Some(last.clone());
    }
    eligible
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_prefix_cannot_starve_later_install_or_cancellation_at_slow_tick_cadence() {
        let mut state = State::default();
        let mut now = Instant::now();
        for index in 0..12 {
            let mut entry = Entry::new();
            entry.next = now;
            if index == 11 {
                entry.step = Step::Stop;
            }
            state
                .entries
                .insert(("owner".into(), format!("{index:02}")), entry);
        }
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let admitted = ready(&mut state, now, JOBS);
            assert_eq!(admitted.len(), JOBS);
            for key in admitted {
                seen.insert(key.clone());
                state.entries.get_mut(&key).unwrap().next = now + RETRY;
            }
            // Every retry is due at the next real kernel tick: backoff alone
            // would repeatedly select the same lexical prefix.
            now += Duration::from_secs(1);
        }
        assert_eq!(seen.len(), 12);
        assert!(seen.contains(&("owner".into(), "11".into())));
        assert_eq!(
            ready(&mut state, now, 1),
            vec![("owner".into(), "00".into())]
        );
    }
}
