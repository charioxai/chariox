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
            let message=format!("Install this App with the declared capabilities?\n\n{}\n\nDeclared information sets are shown for review only. This decision does not grant information-set access.",
                serde_json::to_string_pretty(prompt.challenge.review()).unwrap_or_default());
            let interaction = RuntimeInteraction::for_kernel_operation(
                prompt.challenge.interaction_id(),
                format!("install:{}", prompt.challenge.installation_id()),
                "Install App",
                message,
                vec![
                    RuntimeInteractionChoice::new("approve", "Install", "approve", None),
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
