use super::*;
use crate::runtime::process_resources::{ProcessSample, SessionProcessState};

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

fn row(pid: u32, command: &str, written: u64, deleted_open: u64, outlived: bool) -> SessionProcess {
    SessionProcess {
        owner: SessionProcessOwner::Agent("agent-1".into()),
        process: ProcessSample {
            process_id: format!("process-{pid}"),
            pid,
            command: command.into(),
            resident_set_bytes: 0,
            cpu_millis: 0,
            disk_io_bytes: Some(written),
            disk_written_bytes: Some(written),
            deleted_open_bytes: deleted_open,
            outlived,
            sampled_at_ms: 0,
            age_ms: 0,
        },
        state: SessionProcessState::Running,
    }
}

fn holding() -> Holding {
    Holding {
        session_id: "room-1".into(),
        agent_id: "agent-1".into(),
        processes: vec![
            row(41, "python3 fill.py", 3 * GIB, 2 * GIB, true),
            row(42, "sleep 600", 0, 0, false),
        ],
        temp: Some(PathBuf::from("/h/t/room0001/agent001")),
        temp_bytes: 800 * MIB,
        temp_biggest: vec![(PathBuf::from("/h/t/room0001/agent001/cache"), 500 * MIB)],
        workspace: Some(PathBuf::from("/work/repo")),
        workspace_biggest: vec![(PathBuf::from("/work/repo/target"), 4 * GIB)],
    }
}

#[test]
fn the_ask_names_what_the_agent_holds_so_it_can_stop_leftovers() {
    let text = ask_text(&holding(), GIB, 10 * GIB);
    assert!(text.contains("1 GB free"), "{text}");
    assert!(
        text.contains("pid 41 `python3 fill.py`: wrote 3 GB"),
        "{text}"
    );
    assert!(text.contains("keeps 2 GB in deleted files open"), "{text}");
    assert!(
        text.contains("still running after the command that started it ended"),
        "{text}"
    );
    assert!(
        !text.contains("sleep 600"),
        "a process that wrote nothing is not listed: {text}"
    );
    assert!(
        text.contains("$TMPDIR (/h/t/room0001/agent001): 800 MB; biggest: cache (500 MB)"),
        "{text}"
    );
    assert!(text.contains("target (4 GB)"), "{text}");
    assert!(text.contains("Never delete results or source"), "{text}");
}

#[test]
fn holders_are_ranked_by_what_their_processes_wrote_hold_deleted_and_keep_in_temp() {
    assert_eq!(holding().used(), 3 * GIB + 2 * GIB + 800 * MIB);
}

#[test]
fn just_under_the_floor_the_ask_does_not_say_ten_gigabytes_below_ten_gigabytes() {
    let text = ask_text(&holding(), 10 * GIB - 40 * MIB, 10 * GIB);
    assert!(text.contains("10200 MB free, below the 10240 MB"), "{text}");
}
