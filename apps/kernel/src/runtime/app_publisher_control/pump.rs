use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
use sha2::{Digest, Sha256};
struct PumpGuard<'a>(&'a AtomicBool);
impl Drop for PumpGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn spawn(
    state: &mut State,
    shared: Arc<Shared>,
    key: Key,
    cancelled: Arc<AtomicBool>,
    job: jobs::Job,
) {
    let identity = key.clone();
    let returned = key.clone();
    let handle = state
        .jobs
        .spawn(async move { (returned, jobs::run(shared, key, cancelled, job).await) });
    state.job_keys.insert(handle.id(), identity);
}

// Selection stays bounded by 32 entries and rotates after each admitted key.
pub(super) fn ready_keys(state: &State, capacity: usize) -> Vec<Key> {
    let mut keys = state
        .entries
        .iter()
        .filter(|(_, entry)| {
            !entry.busy
                && Instant::now() >= entry.next
                && !matches!(entry.step, Step::Waiting { .. } | Step::Done)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(cursor) = &state.cursor {
        let split = keys.partition_point(|key| key <= cursor);
        keys.rotate_left(split);
    }
    keys.truncate(capacity);
    keys
}

impl AppPublisherControl {
    pub(crate) async fn pump(&self, runtime: &KernelRuntimeState) {
        if self.0.stopped.load(Ordering::Acquire)
            || self.0.shared.store.require_writer_healthy().is_err()
            || self
                .0
                .pumping
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let _guard = PumpGuard(&self.0.pumping);
        let mut prompts = Vec::new();
        let mut close = Vec::new();
        {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.0.stopped.load(Ordering::Acquire) {
                return;
            }
            while state.requests.try_join_next().is_some() {}
            while let Some(result) = state.jobs.try_join_next_with_id() {
                let (key, result) = match result {
                    Ok((id, (key, result))) => {
                        state.job_keys.remove(&id);
                        (key, result)
                    }
                    Err(error) => {
                        let Some(key) = state.job_keys.remove(&error.id()) else {
                            continue;
                        };
                        (key, Err(jobs::Error::Unknown))
                    }
                };
                if key.0.is_empty() {
                    state.scanning = false;
                    if let Ok(jobs::Outcome::Scan(keys)) = result {
                        state.scan_cursor = if keys.len() == 8 {
                            keys.last().cloned()
                        } else {
                            None
                        };
                        for key in keys {
                            if state.entries.len() < OWNERS {
                                state.entries.entry(key).or_insert_with(Entry::new);
                            }
                        }
                    }
                    continue;
                }
                let Some(entry) = state.entries.get_mut(&key) else {
                    continue;
                };
                entry.busy = false;
                if entry.cancelled.load(Ordering::Acquire) {
                    entry.step = Step::Cancel;
                }
                match result {
                    Ok(
                        jobs::Outcome::Terminal
                        | jobs::Outcome::Review(PublisherReview::Terminal(_)),
                    ) => entry.step = Step::Done,
                    Ok(jobs::Outcome::Review(PublisherReview::Prompt(challenge)))
                        if !entry.cancelled.load(Ordering::Acquire) =>
                    {
                        entry.busy = true;
                        prompts.push((key, Arc::new(challenge)));
                    }
                    Ok(_) => {}
                    Err(jobs::Error::Busy | jobs::Error::Storage) => {
                        entry.next = Instant::now() + RETRY
                    }
                    Err(jobs::Error::Unknown) => {
                        entry.step = if entry.cancelled.load(Ordering::Acquire) {
                            Step::Cancel
                        } else {
                            Step::Arm
                        };
                        entry.next = Instant::now() + RETRY;
                    }
                    Err(jobs::Error::Stopped) => {
                        entry.step = Step::Cancel;
                        entry.next = Instant::now() + RETRY;
                    }
                }
            }
            for entry in state.entries.values_mut() {
                if let Step::Waiting {
                    challenge,
                    receiver,
                } = &mut entry.step
                {
                    let decision = receiver.try_recv();
                    if entry.cancelled.load(Ordering::Acquire)
                        || Instant::now() >= challenge.deadline()
                        || matches!(decision, Err(oneshot::error::TryRecvError::Closed))
                    {
                        close.push((
                            challenge.input().session_id.clone(),
                            challenge.interaction_id().to_owned(),
                        ));
                        entry.step = Step::Cancel;
                    } else if let Ok(value) = decision {
                        let accepted = value.status == "answered"
                            && value.choice_id.as_deref() == Some("approve")
                            && value.reply.as_deref() == Some("approve");
                        entry.step = Step::Decide {
                            challenge: challenge.clone(),
                            accepted,
                        };
                    }
                } else if !entry.busy
                    && entry.cancelled.load(Ordering::Acquire)
                    && !matches!(entry.step, Step::Done)
                {
                    entry.step = Step::Cancel;
                }
            }
            state
                .entries
                .retain(|_, entry| entry.busy || !matches!(entry.step, Step::Done));
            // A due scan takes one slot before active requests can monopolize it.
            if !state.scanning
                && Instant::now() >= state.scan_next
                && state.entries.len() < OWNERS
                && state.jobs.len() + state.requests.len() < JOBS
            {
                state.scanning = true;
                state.scan_next = Instant::now() + Duration::from_secs(1);
                let cursor = state.scan_cursor.clone();
                spawn(
                    &mut state,
                    self.0.shared.clone(),
                    ("".into(), "".into()),
                    Arc::new(AtomicBool::new(false)),
                    jobs::Job::Scan(cursor),
                );
            }
            let keys = ready_keys(
                &state,
                JOBS.saturating_sub(state.jobs.len() + state.requests.len()),
            );
            for key in keys {
                let entry = state.entries.get_mut(&key).unwrap();
                let job = match &entry.step {
                    Step::Arm => jobs::Job::Arm,
                    Step::Decide {
                        challenge,
                        accepted,
                    } => jobs::Job::Decide {
                        challenge: challenge.clone(),
                        accepted: *accepted,
                    },
                    Step::Cancel => jobs::Job::Cancel,
                    _ => continue,
                };
                entry.busy = true;
                let cancelled = entry.cancelled.clone();
                state.cursor = Some(key.clone());
                spawn(&mut state, self.0.shared.clone(), key, cancelled, job);
            }
        }
        for (session, id) in close {
            let _ = runtime.timeout_runtime_interaction(&session, &id).await;
        }
        for (key, challenge) in prompts {
            if self.0.stopped.load(Ordering::Acquire) {
                break;
            }
            let session = &challenge.input().session_id;
            if Instant::now() >= challenge.deadline()
                || !runtime.app_install_session_member(session, &key.0)
            {
                let mut state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.busy = false;
                    entry.step = Step::Cancel;
                }
                continue;
            }
            let input = challenge.input();
            let mut subject = Sha256::new();
            subject.update(key.0.as_bytes());
            subject.update([0]);
            subject.update(key.1.as_bytes());
            let message=format!("Enroll this publisher key for your Chariox Apps?\n\nPublisher: {}\nKey: {}\nPublic key: {}\nSHA-256 fingerprint: {:x}\nExpected trust revision: {}\n\nThis enrolls the exact publisher key. Installing an App still requires its separate capability decision.",
                input.publisher_id,input.key_id,input.public_key.iter().map(|b|format!("{b:02x}")).collect::<String>(),Sha256::digest(input.public_key),input.expected_revision);
            let interaction = RuntimeInteraction::for_kernel_operation(
                challenge.interaction_id(),
                format!("publisher_enroll:{:x}", subject.finalize()),
                "Enroll App publisher",
                message,
                vec![
                    RuntimeInteractionChoice::new("approve", "Enroll", "approve", None),
                    RuntimeInteractionChoice::new("decline", "Cancel", "decline", None),
                ],
            );
            let result = runtime
                .create_kernel_operation_interaction(session, &key.0, interaction)
                .await;
            let mut abandoned = false;
            {
                let mut state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.busy = false;
                    if self.0.stopped.load(Ordering::Acquire)
                        || entry.cancelled.load(Ordering::Acquire)
                    {
                        abandoned = result.is_ok();
                        entry.step = Step::Cancel;
                    } else {
                        match result {
                            Ok(receiver) => {
                                entry.step = Step::Waiting {
                                    challenge: challenge.clone(),
                                    receiver,
                                }
                            }
                            Err(_) => {
                                entry.step = Step::Arm;
                                entry.next = Instant::now() + RETRY;
                            }
                        }
                    }
                } else {
                    abandoned = result.is_ok();
                }
            }
            if abandoned {
                let _ = runtime
                    .timeout_runtime_interaction(session, challenge.interaction_id())
                    .await;
            }
        }
    }
}
