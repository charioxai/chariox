//! One bounded pass per existing kernel tick; human waits hold no App permits.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
struct PumpGuard<'a>(&'a AtomicBool);
impl Drop for PumpGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Prompt {
    key: Key,
    session: String,
    challenge: Arc<InstallApprovalChallenge>,
}

fn spawn(
    state: &mut State,
    shared: Arc<Shared>,
    key: Key,
    cancelled: Arc<AtomicBool>,
    job: jobs::Job,
) {
    let task_key = key.clone();
    let handle = state.tasks.spawn(async move {
        let result = jobs::run(shared, task_key.clone(), cancelled, job).await;
        (task_key, result)
    });
    state.task_keys.insert(handle.id(), key);
}
impl AppInstallControl {
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
        let mut waits = Vec::new();
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
            while state.eviction.try_join_next().is_some() {}
            let mut evict_for = None;
            for _ in 0..8 {
                let Some(joined) = state.tasks.try_join_next_with_id() else {
                    break;
                };
                let (key, result) = match joined {
                    Ok((id, (key, result))) => {
                        state.task_keys.remove(&id);
                        (key, result)
                    }
                    Err(error) => {
                        let Some(key) = state.task_keys.remove(&error.id()) else {
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
                if entry.cancelled.load(Ordering::Acquire)
                    && !matches!(entry.step, Step::Stop | Step::Done)
                {
                    entry.step = Step::Stop;
                }
                match result {
                    Ok(jobs::Outcome::Review { operation, review })
                        if !entry.cancelled.load(Ordering::Acquire) =>
                    {
                        match review {
                            InstallReviewDisposition::Approved => entry.step = Step::Start,
                            InstallReviewDisposition::Terminal => entry.step = Step::Done,
                            InstallReviewDisposition::Prompt(challenge) => {
                                if let Some(input) = operation.input {
                                    entry.busy = true;
                                    prompts.push(Prompt {
                                        key,
                                        session: input.session_id,
                                        challenge: Arc::new(challenge),
                                    });
                                } else {
                                    entry.step = Step::Fail("app_install_input_missing");
                                }
                            }
                        }
                    }
                    Ok(jobs::Outcome::Stopped) => entry.step = Step::Done,
                    Ok(jobs::Outcome::Done) => {
                        // Cancellation may have replaced a Work/Decision phase
                        // while it ran. Its final completion cannot erase Stop.
                        if !matches!(entry.step, Step::Stop) {
                            entry.step = Step::Done;
                        }
                    }
                    Ok(_) => {}
                    Err(jobs::Error::Busy | jobs::Error::Storage) => {
                        entry.next = Instant::now() + RETRY
                    }
                    // An installed App's first start at the live-worker limit
                    // makes room as a user or on-demand start does, instead of
                    // retrying until some worker happens to stop.
                    Err(jobs::Error::LiveLimit) => {
                        if let Some(first) = wait_for_slot(entry, Instant::now()) {
                            if first {
                                waits.push(key.clone());
                            }
                            evict_for.get_or_insert_with(|| key.0.clone());
                        }
                    }
                    Err(jobs::Error::Unknown) => {
                        entry.step = if entry.cancelled.load(Ordering::Acquire) {
                            Step::Stop
                        } else {
                            Step::Work
                        };
                        entry.next = Instant::now() + RETRY;
                    }
                    Err(jobs::Error::Failed(code)) => {
                        entry.step = if entry.cancelled.load(Ordering::Acquire) {
                            Step::Stop
                        } else {
                            Step::Fail(code)
                        };
                        entry.next = Instant::now() + RETRY;
                    }
                }
            }
            for entry in state.entries.values_mut() {
                if let Step::Waiting {
                    session,
                    challenge,
                    receiver,
                    deadline,
                } = &mut entry.step
                {
                    let decision = receiver.try_recv();
                    let expired =
                        entry.cancelled.load(Ordering::Acquire) || Instant::now() >= *deadline;
                    if expired || matches!(decision, Err(oneshot::error::TryRecvError::Closed)) {
                        close.push((session.clone(), challenge.interaction_id().to_owned()));
                        entry.step = if entry.cancelled.load(Ordering::Acquire) {
                            Step::Stop
                        } else {
                            Step::Fail("app_install_approval_expired")
                        };
                    } else if let Ok(value) = decision {
                        let accepted = value.status == "answered"
                            && value.choice_id.as_deref() == Some("approve")
                            && value.reply.as_deref() == Some("approve");
                        entry.step = Step::Decide {
                            challenge: challenge.clone(),
                            accepted,
                            deadline: *deadline,
                        };
                    }
                }
            }
            if let Some(owner) = evict_for {
                let runtime = runtime.clone();
                // The installing App has no live worker yet, so no target is
                // spared. A pass that finds one running leaves it to finish.
                schedule_eviction(&mut state.eviction, async move {
                    runtime.evict_idle_app(&owner, "").await
                });
            }
            state
                .entries
                .retain(|_, entry| entry.busy || !matches!(entry.step, Step::Done));
            if !state.scanning
                && Instant::now() >= state.scan_next
                && state.tasks.len() < JOBS
                && state.entries.len() < OWNERS
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
            let capacity = JOBS.saturating_sub(state.tasks.len());
            let keys = selection::ready(&mut state, Instant::now(), capacity);
            for key in keys {
                let entry = state.entries.get_mut(&key).unwrap();
                let job = match &entry.step {
                    Step::Work => jobs::Job::Work,
                    Step::Decide {
                        challenge,
                        accepted,
                        deadline,
                    } => jobs::Job::Decide {
                        challenge: challenge.clone(),
                        accepted: *accepted,
                        deadline: *deadline,
                    },
                    Step::Start => jobs::Job::Start,
                    Step::Stop => jobs::Job::Stop,
                    Step::Fail(code) => jobs::Job::Fail(code),
                    _ => continue,
                };
                let cancelled = entry.cancelled.clone();
                entry.busy = true;
                spawn(&mut state, self.0.shared.clone(), key, cancelled, job);
            }
        }
        for (session, id) in close {
            let _ = runtime.timeout_runtime_interaction(&session, &id).await;
        }
        for (owner, request_id) in waits {
            crate::logging::info_with_fields(
                "app.install",
                "App install waits for a live worker slot; stopping an idle worker",
                serde_json::json!({ "owner_id": owner, "request_id": request_id }),
            );
        }
        for prompt in prompts {
            if self.0.stopped.load(Ordering::Acquire) {
                break;
            }
            if !runtime.app_install_session_member(&prompt.session, &prompt.key.0) {
                let mut state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(entry) = state.entries.get_mut(&prompt.key) {
                    entry.busy = false;
                    entry.step = Step::Fail("app_install_session_unavailable");
                }
                continue;
            }
            let (title, question, action) = if prompt.challenge.is_reinstall() {
                ("Reinstall App", "Reinstall this App into its kept data with the declared capabilities? Its earlier automations, inbox routes and connection grants are not restored.", "Reinstall")
            } else if prompt.challenge.is_update() {
                ("Update App", "Replace the installed App with this release and its declared capabilities? Its data is kept.", "Update")
            } else {
                (
                    "Install App",
                    "Install this App with the declared capabilities?",
                    "Install",
                )
            };
            let message=format!("{question}\n\n{}\n\nDeclared information sets are shown for review only. This decision does not grant information-set access.",
                serde_json::to_string_pretty(prompt.challenge.review()).unwrap_or_default());
            let interaction = RuntimeInteraction::for_kernel_operation(
                prompt.challenge.interaction_id(),
                format!("install:{}", prompt.challenge.installation_id()),
                title,
                message,
                vec![
                    RuntimeInteractionChoice::new("approve", action, "approve", None),
                    RuntimeInteractionChoice::new("decline", "Cancel", "decline", None),
                ],
            );
            let result = runtime
                .create_kernel_operation_interaction(&prompt.session, &prompt.key.0, interaction)
                .await;
            let mut abandoned = false;
            {
                let mut state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(entry) = state.entries.get_mut(&prompt.key) {
                    entry.busy = false;
                    if self.0.stopped.load(Ordering::Acquire)
                        || entry.cancelled.load(Ordering::Acquire)
                    {
                        abandoned = result.is_ok();
                    } else {
                        match result {
                            Ok(receiver) => {
                                entry.step = Step::Waiting {
                                    session: prompt.session.clone(),
                                    deadline: prompt.challenge.deadline(),
                                    challenge: prompt.challenge.clone(),
                                    receiver,
                                }
                            }
                            Err(_) => {
                                entry.step = Step::Work;
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
                    .timeout_runtime_interaction(&prompt.session, prompt.challenge.interaction_id())
                    .await;
            }
        }
    }
}

/// A first start that met the live-worker limit retries later and, unless the
/// install was cancelled meanwhile, wants an idle worker evicted. `Some(true)`
/// the first time, so the wait is logged once; `None` wants no eviction.
fn wait_for_slot(entry: &mut Entry, now: Instant) -> Option<bool> {
    entry.next = now + RETRY;
    if entry.cancelled.load(Ordering::Acquire) || !matches!(entry.step, Step::Start) {
        return None;
    }
    Some(!std::mem::replace(&mut entry.waited_for_slot, true))
}

/// Stopping a worker joins its thread, so it never runs on the tick: one
/// eviction at a time runs beside it, and a later pass schedules the next.
fn schedule_eviction(
    eviction: &mut JoinSet<()>,
    task: impl std::future::Future<Output = ()> + Send + 'static,
) -> bool {
    if !eviction.is_empty() {
        return false;
    }
    eviction.spawn(task);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_at_the_live_limit_evicts_and_logs_once_unless_cancelled() {
        let now = Instant::now();
        let mut entry = Entry::new();
        entry.step = Step::Start;
        assert_eq!(wait_for_slot(&mut entry, now), Some(true));
        assert_eq!(entry.next, now + RETRY);
        assert_eq!(wait_for_slot(&mut entry, now), Some(false));
        // A cancelled install never stops another App's worker.
        entry.cancelled.store(true, Ordering::Release);
        assert_eq!(wait_for_slot(&mut entry, now), None);
        let mut stopping = Entry::new();
        stopping.step = Step::Stop;
        assert_eq!(wait_for_slot(&mut stopping, now), None);
        assert_eq!(stopping.next, now + RETRY);
    }

    #[tokio::test]
    async fn only_one_eviction_runs_at_a_time() {
        let mut eviction = JoinSet::new();
        let (release, wait) = oneshot::channel::<()>();
        assert!(schedule_eviction(&mut eviction, async move {
            let _ = wait.await;
        }));
        assert!(!schedule_eviction(&mut eviction, async {}));
        release.send(()).unwrap();
        while eviction.join_next().await.is_some() {}
        assert!(schedule_eviction(&mut eviction, async {}));
    }
}
