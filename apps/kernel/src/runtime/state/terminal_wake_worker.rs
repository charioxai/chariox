//! MP-08 / MP-10 / MP-11 A03: a terminal wake for a terminal that runs on the
//! machine an agent is leased to. The worker kernel owns the terminal and runs
//! the watch (the capped `Wait` of the existing Room terminal peer request);
//! the home only repeats that request until a watch fires, then reports the one
//! occurrence. No new relay shape, prompt or permission path.
use crate::room_terminal::RoomTerminalWaitResult;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// How long the worker may stay unreachable before the wake is reported lost.
const UNREACHABLE_FOR: Duration = Duration::from_secs(120);
const RETRY_AFTER: Duration = Duration::from_secs(2);

/// One capped wait on the worker.
pub(super) enum Step {
    Waited(Box<RoomTerminalWaitResult>),
    /// The worker did not answer; the terminal may be fine.
    Unreachable(String),
    /// The terminal can no longer be reached by this agent (lease replaced).
    Gone(String),
}

pub(super) enum Settled {
    Fired(Box<RoomTerminalWaitResult>),
    Lost(String),
}

/// How far a capped wait is taken back, so a watched text that straddles the
/// cap is still seen whole.
#[derive(Clone, Copy)]
pub(super) struct Overlap {
    /// Output before this was seen by whoever armed the wake.
    pub floor: u64,
    pub bytes: u64,
}

impl Overlap {
    /// Matching reads the plain text, so escape sequences between the
    /// characters of a text take raw bytes of their own.
    pub(super) fn for_texts<'a>(floor: u64, texts: impl Iterator<Item = &'a str>) -> Self {
        let longest = texts.map(str::len).max().unwrap_or(0) as u64;
        Self {
            floor,
            bytes: longest.saturating_mul(4).saturating_add(256),
        }
    }

    /// Where the wait after one that ended at `offset` starts. A text that
    /// lay wholly before `offset` already fired, so the replay cannot repeat
    /// one; it can only finish a text the cap cut in two.
    fn next_start(self, offset: u64) -> u64 {
        offset.saturating_sub(self.bytes).max(self.floor)
    }
}

/// Repeats `wait_once` until a watch fires or the worker is gone. It is given
/// the output offset of the last answer (taken back by `overlap`), so output
/// printed between two capped waits is still matched, also when a watched text
/// is split across them.
pub(super) async fn supervise<F, Fut>(
    first_offset: Option<u64>,
    overlap: Overlap,
    mut wait_once: F,
) -> Settled
where
    F: FnMut(Option<u64>) -> Fut,
    Fut: Future<Output = Step>,
{
    let mut offset = first_offset;
    let mut last_answer = Instant::now();
    loop {
        match wait_once(offset).await {
            Step::Waited(waited) if waited.fired.is_some() => return Settled::Fired(waited),
            Step::Waited(capped) => {
                offset = Some(overlap.next_start(capped.output_offset));
                last_answer = Instant::now();
            }
            Step::Gone(reason) => return Settled::Lost(reason),
            Step::Unreachable(reason) => {
                if last_answer.elapsed() >= UNREACHABLE_FOR {
                    return Settled::Lost(format!(
                        "the worker kernel has not answered for {} s ({reason}); the terminal may still be running there, read it with terminal_read",
                        UNREACHABLE_FOR.as_secs()
                    ));
                }
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
}

/// The supervision tasks of this kernel, by wake id.
#[derive(Default)]
pub(super) struct WorkerWatches {
    tasks: Mutex<HashMap<String, tokio::task::AbortHandle>>,
}

impl WorkerWatches {
    /// Spawns `task` under `wake_id`; `done` removes it again when it ends.
    pub(super) fn spawn(
        self: &std::sync::Arc<Self>,
        wake_id: String,
        task: impl Future<Output = ()> + Send + 'static,
    ) {
        let this = self.clone();
        let id = wake_id.clone();
        let mut tasks = self.tasks.lock().expect("worker watches poisoned");
        let handle = tokio::spawn(async move {
            task.await;
            this.tasks
                .lock()
                .expect("worker watches poisoned")
                .remove(&id);
        });
        tasks.insert(wake_id, handle.abort_handle());
    }

    /// Stops supervising; the worker's own capped wait ends by itself.
    pub(super) fn stop(&self, wake_id: &str) {
        if let Some(handle) = self
            .tasks
            .lock()
            .expect("worker watches poisoned")
            .remove(wake_id)
        {
            handle.abort();
        }
    }

    pub(super) fn is_supervised(&self, wake_id: &str) -> bool {
        self.tasks
            .lock()
            .expect("worker watches poisoned")
            .contains_key(wake_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_terminal::{
        RoomTerminalDriver, RoomTerminalEventKind, RoomTerminalStatus, RoomTerminalSummary,
        TerminalActor,
    };
    use std::sync::{Arc, Mutex};

    const NONE: Overlap = Overlap { floor: 0, bytes: 0 };

    fn answer(fired: Option<RoomTerminalEventKind>, output_offset: u64) -> RoomTerminalWaitResult {
        RoomTerminalWaitResult {
            fired,
            terminal: RoomTerminalSummary {
                terminal_id: "term-1".into(),
                session_id: "backing".into(),
                title: "build".into(),
                program: "bash".into(),
                working_directory: "/w".into(),
                cols: 80,
                rows: 24,
                status: RoomTerminalStatus::Running,
                driver: RoomTerminalDriver::Free,
                opened_by: TerminalActor::User,
                user_interacted: false,
                created_at_ms: 0,
                output_offset,
                dropped_input_bytes: 0,
                host_agent: None,
            },
            screen_text: "screen".into(),
            output_offset,
        }
    }

    fn finished(exit_code: u32) -> Option<RoomTerminalEventKind> {
        Some(RoomTerminalEventKind::Finished {
            exit_code: Some(exit_code),
            signal: None,
        })
    }

    #[tokio::test(start_paused = true)]
    async fn capped_waits_repeat_from_the_last_offset_until_a_watch_fires() {
        let offsets = Arc::new(Mutex::new(Vec::new()));
        let mut script = vec![
            Step::Waited(Box::new(answer(None, 10))),
            Step::Waited(Box::new(answer(None, 25))),
            Step::Waited(Box::new(answer(finished(3), 40))),
        ]
        .into_iter();
        let seen = offsets.clone();
        let settled = supervise(Some(4), NONE, |offset| {
            seen.lock().unwrap().push(offset);
            std::future::ready(script.next().expect("no wait after the watch fired"))
        })
        .await;
        let Settled::Fired(waited) = settled else {
            panic!("a fired watch settles the wake");
        };
        assert_eq!(waited.fired, finished(3));
        assert_eq!(*offsets.lock().unwrap(), [Some(4), Some(10), Some(25)]);
    }

    #[test]
    fn the_overlap_never_reaches_below_what_the_wake_was_armed_after() {
        let overlap = Overlap::for_texts(100, ["ready-marker"].into_iter());
        assert_eq!(overlap.next_start(150), 100);
        assert_eq!(overlap.next_start(10_000), 10_000 - overlap.bytes);
        assert_eq!(overlap.next_start(5), 100);
    }

    /// Real PTY: the text is printed in two halves, one on each side of a
    /// capped wait.
    #[tokio::test]
    async fn a_text_split_across_two_capped_waits_still_fires() {
        let root = std::env::temp_dir().join(format!("chariox-split-wait-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let service = crate::room_terminal::RoomTerminalService::new(root.join("terminals"));
        let terminal = service
            .open(crate::room_terminal::OpenRoomTerminal {
                session_id: "room-1".into(),
                working_directory: root.clone(),
                cols: 100,
                rows: 30,
                title: None,
                opened_by: TerminalActor::User,
                command: Some(vec![
                    "sh".into(),
                    "-c".into(),
                    "printf ready-; sleep 2; printf marker; sleep 30".into(),
                ]),
            })
            .unwrap();
        let text = "ready-marker";
        let watches =
            vec![crate::room_terminal::RoomTerminalWatch::OutputContains { text: text.into() }];
        let waits = Arc::new(Mutex::new(0u32));
        let counted = waits.clone();
        let settled = tokio::time::timeout(
            Duration::from_secs(20),
            supervise(
                Some(0),
                Overlap::for_texts(0, [text].into_iter()),
                |offset| {
                    *counted.lock().unwrap() += 1;
                    let service = service.clone();
                    let watches = watches.clone();
                    let terminal_id = terminal.terminal_id.clone();
                    async move {
                        match service
                            .wait(
                                "room-1",
                                &terminal_id,
                                "agent",
                                watches,
                                offset,
                                Duration::from_millis(700),
                            )
                            .await
                        {
                            Ok(waited) => Step::Waited(Box::new(waited)),
                            Err(error) => Step::Gone(error.to_string()),
                        }
                    }
                },
            ),
        )
        .await
        .expect("the split text fires before the test gives up");
        let Settled::Fired(waited) = settled else {
            panic!("the wait must fire, not be lost");
        };
        assert!(matches!(
            waited.fired,
            Some(RoomTerminalEventKind::OutputMatched { .. })
        ));
        assert!(*waits.lock().unwrap() >= 2, "the text crossed a cap");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_worker_is_retried_then_reported_lost() {
        let calls = Arc::new(Mutex::new(0u32));
        let counted = calls.clone();
        let settled = supervise(None, NONE, |_| {
            *counted.lock().unwrap() += 1;
            std::future::ready(Step::Unreachable("relay down".into()))
        })
        .await;
        let Settled::Lost(reason) = settled else {
            panic!("an unreachable worker ends in a definitive outcome");
        };
        assert!(
            reason.contains("120 s") && reason.contains("terminal_read"),
            "{reason}"
        );
        assert!(*calls.lock().unwrap() > 10, "it retried before giving up");
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_that_comes_back_keeps_the_watch() {
        let mut script = vec![
            Step::Unreachable("blip".into()),
            Step::Unreachable("blip".into()),
            Step::Waited(Box::new(answer(finished(0), 7))),
        ]
        .into_iter();
        let settled = supervise(None, NONE, |_| std::future::ready(script.next().unwrap())).await;
        assert!(matches!(settled, Settled::Fired(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_replaced_lease_is_lost_at_once() {
        let settled = supervise(None, NONE, |_| {
            std::future::ready(Step::Gone("lease replaced".into()))
        })
        .await;
        assert!(matches!(settled, Settled::Lost(reason) if reason == "lease replaced"));
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_a_supervised_watch_aborts_it() {
        let watches = Arc::new(WorkerWatches::default());
        watches.spawn("wake-1".into(), std::future::pending());
        assert!(watches.is_supervised("wake-1"));
        watches.stop("wake-1");
        assert!(!watches.is_supervised("wake-1"));
    }
}
