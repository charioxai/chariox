use super::*;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[tokio::test]
async fn a_live_agent_can_first_use_its_temp_root_after_a_sweep() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
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
    if crate::test_support::isolate_environment_test() {
        return;
    }
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
    if crate::test_support::isolate_environment_test() {
        return;
    }
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
