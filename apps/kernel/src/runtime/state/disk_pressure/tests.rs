use super::*;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[tokio::test]
async fn a_live_agent_can_first_use_its_temp_root_after_a_sweep() {
    crate::test_support::isolated_env_test!();
    let state = crate::test_support::allocation::runtime_with_rooms(1);
    let room = state.owned.session_store.list_all_sessions().pop().unwrap();
    let agent = state
        .owned
        .agent_store
        .get_session_agents(room.id())
        .pop()
        .unwrap();
    let environment = crate::agent_temp::environment(room.id(), agent.id());
    let path = PathBuf::from(&environment[0].1);
    std::fs::write(path.join("stale"), b"disposable").unwrap();
    KernelRuntimeState::trim_temp(Duration::ZERO);
    assert!(
        !path.join("stale").exists(),
        "live agents' idle contents are trimmed"
    );
    let first = std::fs::File::create(path.join("first-later-use"));
    assert!(
        first.is_ok(),
        "a live agent's inherited TMPDIR was removed: {first:?}"
    );
}

/// A room whose agent holds `megabytes` of fresh scratch and is busy with its
/// own work, so any prompt the kernel sent it would queue behind that work.
fn busy_holder(state: &KernelRuntimeState, index: usize, megabytes: u64) -> (String, String) {
    let room = state.owned.session_store.list_all_sessions().remove(index);
    let agent = state
        .owned
        .agent_store
        .get_session_agents(room.id())
        .pop()
        .unwrap();
    let environment = crate::agent_temp::environment(room.id(), agent.id());
    let mut file = std::fs::File::create(PathBuf::from(&environment[0].1).join("output")).unwrap();
    use std::io::Write;
    let chunk = vec![1; MIB as usize];
    for _ in 0..megabytes {
        file.write_all(&chunk).unwrap();
    }
    let busy = crate::session::PromptQueueItem::new(
        format!("busy-{index}"),
        "test",
        agent.id(),
        "current work",
        crate::session::PromptStatus::Running,
    );
    state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt(&room, busy, false)
        .unwrap();
    (room.id().to_string(), agent.id().to_string())
}

fn queued_prompts(state: &KernelRuntimeState, room: &str, agent: &str) -> usize {
    let session = state.owned.session_store.get_session(room).unwrap();
    state
        .owned
        .prompt_state_owner
        .state_parts(&session, agent)
        .1
        .len()
}

/// Owner decision 2026-11-09: a full disk is never put to an agent as a prompt.
/// The biggest holders sit well above the old 64 MiB threshold on a volume
/// below its floor, and the trim has nothing to free.
#[tokio::test]
async fn a_volume_below_its_floor_sends_no_prompt_to_any_agent() {
    crate::test_support::isolated_env_test!();
    let mut state = crate::test_support::allocation::runtime_with_rooms(3);
    state.disk_volume_probe = Some(|_| Some((1, GIB, 10 * GIB)));
    let holders: Vec<_> = (0..3)
        .map(|index| busy_holder(&state, index, 96 - index as u64 * 8))
        .collect();
    let mut pace = Pace::default();
    state.tend_disk(&mut pace).await;
    state.tend_disk(&mut Pace::default()).await;
    for (room, agent) in &holders {
        assert_eq!(
            queued_prompts(&state, room, agent),
            0,
            "the kernel must not ask an agent to free disk"
        );
        assert!(
            state
                .owned
                .durable_state_store
                .agent_tasks(Some(room), Some(agent))
                .unwrap()
                .is_empty(),
            "no task is admitted for a disk notice"
        );
    }
}

/// The kernel's own work stays: scratch nothing wrote for the idle window goes,
/// with and without pressure, and what was just written stays.
#[tokio::test]
async fn idle_scratch_is_still_trimmed_with_and_without_pressure() {
    crate::test_support::isolated_env_test!();
    let mut state = crate::test_support::allocation::runtime_with_rooms(2);
    let scratch = |index: usize| {
        let room = state.owned.session_store.list_all_sessions().remove(index);
        let agent = state
            .owned
            .agent_store
            .get_session_agents(room.id())
            .pop()
            .unwrap();
        let environment = crate::agent_temp::environment(room.id(), agent.id());
        let stale = PathBuf::from(&environment[0].1).join("stale");
        std::fs::write(&stale, b"disposable").unwrap();
        let file = std::fs::File::options().write(true).open(&stale).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 3600))
            .unwrap();
        let fresh = stale.with_file_name("fresh");
        std::fs::write(&fresh, b"just written").unwrap();
        (stale, fresh)
    };
    let (calm_stale, calm_fresh) = scratch(0);
    state.tend_disk(&mut Pace::default()).await;
    assert!(
        !calm_stale.exists(),
        "the periodic trim removes idle scratch"
    );
    assert!(calm_fresh.exists(), "fresh scratch stays");
    state.disk_volume_probe = Some(|_| Some((1, GIB, 10 * GIB)));
    let (pressed_stale, pressed_fresh) = scratch(1);
    state.tend_disk(&mut Pace::default()).await;
    assert!(
        !pressed_stale.exists(),
        "the trim under pressure removes idle scratch"
    );
    assert!(pressed_fresh.exists(), "fresh scratch stays under pressure");
}

/// Collects the messages of warnings logged on the current thread.
#[derive(Clone, Default)]
struct Warnings(Arc<std::sync::Mutex<Vec<String>>>);

impl tracing::Subscriber for Warnings {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.lock().unwrap().push(message.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A volume below its floor that the trim cannot help leaves one log line,
/// at most once a minute, and no line while the trim still frees something.
#[tokio::test]
async fn a_volume_below_its_floor_that_the_trim_cannot_help_is_logged_once_a_minute() {
    crate::test_support::isolated_env_test!();
    let mut state = crate::test_support::allocation::runtime_with_rooms(1);
    state.disk_volume_probe = Some(|_| Some((1, GIB, 10 * GIB)));
    let warnings = Warnings::default();
    let _logging = tracing::subscriber::set_default(warnings.clone());
    let lines = || {
        warnings
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("free-space floor"))
            .count()
    };
    let (room, agent) = busy_holder(&state, 0, 1);
    let mut pace = Pace::default();
    state.tend_disk(&mut pace).await;
    state.tend_disk(&mut pace).await;
    assert_eq!(lines(), 1, "one line, however often the disk is checked");
    pace.logged = Instant::now().checked_sub(LOG_EVERY + Duration::from_secs(1));
    state.tend_disk(&mut pace).await;
    assert_eq!(lines(), 2, "a minute later the volume is logged again");
    let environment = crate::agent_temp::environment(&room, &agent);
    let stale = PathBuf::from(&environment[0].1).join("stale");
    std::fs::write(&stale, b"disposable").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 3600))
        .unwrap();
    pace.logged = Instant::now().checked_sub(LOG_EVERY + Duration::from_secs(1));
    state.tend_disk(&mut pace).await;
    assert!(!stale.exists());
    assert_eq!(lines(), 2, "no line when the trim freed something");
}
