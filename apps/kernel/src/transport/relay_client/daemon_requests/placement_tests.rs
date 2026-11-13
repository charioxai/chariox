//! A held launch placed on a second kernel through the production `GatewayExecPlacement`.
//! The second kernel is a real router and runtime state that receives the exact envelope
//! the gateway would send (`dispatch_on_behalf_of`: its own authorization, ledger and
//! Room exec service). Not exercised here: the relay hop, the encrypted link and
//! `forward_audited` itself.
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;

use super::*;
use crate::local::{
    LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult,
};
use crate::room_exec::gateway_placement::{GatewayExecPlacement, GrantedSend, OriginGrant};
use crate::room_exec::queue_tests::{limited, queued, TempDir};
use crate::room_exec::{ExecOutcome, ExecState, ExecStream, Placement, Requester, RoomExecService};
use crate::runtime::kernel_gateway::{origins, GatewayError};
use crate::runtime::process_resources::SessionProcessOwner;

const GATEWAY_KEY: &str = "kernel-a-gateway-key";
const ORIGIN_NAME: &str = "kernel-a";

/// What the gateway link does to one send.
#[derive(Clone, Copy)]
enum Fault {
    /// The target runs the request, then the answer is lost.
    LoseReply,
    /// The request never reaches the target.
    DropRequest,
}

struct SecondKernel {
    router: CommandRouter,
    session_id: String,
    owner: String,
    cache: CommandResultCache,
    faults: Mutex<VecDeque<Fault>>,
    sent: Mutex<Vec<String>>,
    _root: TempDir,
    _workspace: crate::test_support::TestWorktree,
}

impl SecondKernel {
    fn new(faults: &[Fault]) -> Arc<Self> {
        let root = TempDir::new();
        let workspace = crate::test_support::TestWorktree::new("placement-second-kernel");
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config.providers.default = Some("dev-stub".into());
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.path().join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.path().join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.path().join("artifacts.db").display().to_string());
        let config = config.with_session_history_root(root.path().join("sessions"));
        let state_root = config.private_runtime_state_root();
        let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let router =
            CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
        origins::admit(
            &origins::path_in(&state_root),
            GATEWAY_KEY,
            Some(origins::Named {
                kernel: "kernel-a-id",
                name: Some(ORIGIN_NAME),
                harness: "claude",
            }),
            crate::session::unix_epoch_ms(),
        )
        .unwrap();
        Arc::new(Self {
            owner: session.owner_user_id().to_string(),
            session_id: session.id().to_string(),
            router,
            cache: CommandResultCache::default(),
            faults: Mutex::new(faults.iter().copied().collect()),
            sent: Mutex::new(Vec::new()),
            _root: root,
            _workspace: workspace,
        })
    }

    fn placement(self: &Arc<Self>) -> Arc<GatewayExecPlacement> {
        Arc::new(GatewayExecPlacement::new(
            Arc::clone(self) as Arc<dyn GrantedSend>,
            self.session_id.clone(),
        ))
    }

    async fn dispatch(
        &self,
        command_id: String,
        request: &LocalDaemonRequest,
    ) -> Result<Value, GatewayError> {
        // The envelope as the gateway serializes it.
        let wire: LocalDaemonRequest =
            serde_json::from_value(serde_json::to_value(request).unwrap()).unwrap();
        let on_behalf_of = crate::local::OnBehalfOf {
            origin_kernel: "kernel-a-id".into(),
            origin_name: Some(ORIGIN_NAME.into()),
            grant_id: "reach:run-1".into(),
            harness: "claude".into(),
        };
        let outcome = dispatch_on_behalf_of(
            &self.router,
            &AtomicU64::new(1),
            KernelCaller::for_relay_request(Some(RelayCallerIdentity {
                realm_id: "test".into(),
                subject: "kernel-a-gateway".into(),
                subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
                expires_at_ms: u64::MAX,
                token_id: None,
                user_id: Some(self.owner.clone()),
                public_key_thumbprint: None,
            })),
            wire,
            Some(command_id),
            &self.cache,
            &on_behalf_of,
            GATEWAY_KEY,
        )
        .await;
        match outcome {
            RelayDispatchOutcome::Response(value) => Ok(value),
            RelayDispatchOutcome::RelayError(error) => Err(GatewayError::Remote {
                name: "kernel-b".into(),
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            }),
        }
    }

    async fn terminal(
        &self,
        acting_agent: Option<&str>,
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, GatewayError> {
        let request = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
            session_id: self.session_id.clone(),
            acting_agent: acting_agent.map(str::to_owned),
            host_agent: None,
            operation,
        });
        let answer = self
            .dispatch(format!("probe:{}", rand::random::<u64>()), &request)
            .await?;
        match serde_json::from_value(answer).unwrap() {
            LocalDaemonResponse::RoomTerminal { result } => Ok(result),
            other => panic!("not a Room terminal answer: {other:?}"),
        }
    }

    async fn execs(&self) -> Vec<crate::room_exec::ExecSummary> {
        match self
            .terminal(None, RoomTerminalOperation::ExecList)
            .await
            .unwrap()
        {
            RoomTerminalResult::ExecListed { execs } => execs,
            other => panic!("{other:?}"),
        }
    }

    fn open_command_ids(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|id| id.ends_with(":open"))
            .cloned()
            .collect()
    }
}

impl GrantedSend for SecondKernel {
    fn send<'a>(
        &'a self,
        command_id: String,
        request: &'a LocalDaemonRequest,
    ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(command_id.clone());
            let fault = if command_id.ends_with(":open") || command_id.ends_with(":room") {
                self.faults.lock().unwrap().pop_front()
            } else {
                None
            };
            let lost = || GatewayError::ConnectionLost {
                name: "kernel-b".into(),
            };
            match fault {
                Some(Fault::DropRequest) => Err(lost()),
                Some(Fault::LoseReply) => {
                    self.dispatch(command_id, request).await?;
                    Err(lost())
                }
                None => self.dispatch(command_id, request).await,
            }
        })
    }
}

fn hold(home: &RoomExecService, cwd: &Path, script: &str, agent: Option<&str>) -> String {
    home.open_as(
        "room",
        crate::session::DEFAULT_LOCAL_USER_ID,
        cwd,
        &["/bin/bash".into(), "-c".into(), script.into()],
        None,
        agent.map(str::to_owned),
    )
    .unwrap()
}

async fn collect(service: &RoomExecService, id: &str) -> (Vec<u8>, Vec<u8>, ExecOutcome) {
    let (mut cursor, mut stdout, mut stderr) = (0, Vec::new(), Vec::new());
    loop {
        let page = service
            .read("room", id, cursor, Duration::from_secs(5))
            .await
            .unwrap();
        for chunk in page.chunks {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(chunk.data_base64)
                .unwrap();
            match chunk.stream {
                ExecStream::Stdout => stdout.extend(bytes),
                ExecStream::Stderr => stderr.extend(bytes),
            }
        }
        cursor = page.next_cursor;
        if let Some(outcome) = page.outcome {
            return (stdout, stderr, outcome);
        }
    }
}

async fn eventually<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(found) = probe() {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition not reached")
}

// MP-08 / MP-09 / MP-10 / MP-11: the whole launch goes through the forwarded Room exec
// operations; the target shows the origin kernel's agent, and the holding kernel's row
// follows the target's outcome.
#[tokio::test(flavor = "multi_thread")]
async fn a_placed_launch_runs_through_the_gateway_and_shows_the_origin_agent() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(
        &home,
        home_dir.path(),
        "cat; printf '\\377err' >&2; exit 7",
        Some("agent-1"),
    );
    queued(&home, &id).await;

    home.place("room", &id, ORIGIN_NAME, second.placement())
        .unwrap();
    let input = b"\x00bytes\xff";
    let encoded = base64::engine::general_purpose::STANDARD.encode(input);
    assert_eq!(
        home.write("room", &id, 0, &encoded).unwrap(),
        input.len() as u64
    );

    // While it runs, the process list of the target names the agent of the origin kernel.
    let rows = eventually(|| {
        let rows = second
            .router
            .runtime_state()
            .session_processes(&second.session_id);
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert!(
        rows.iter()
            .all(|row| row.owner == SessionProcessOwner::Agent("kernel-a/agent-1".into())),
        "every process of the launch is the origin kernel's agent: {rows:?}"
    );
    let remote = second.execs().await;
    assert_eq!(
        remote[0].owner,
        SessionProcessOwner::Agent("kernel-a/agent-1".into())
    );
    assert_eq!(
        home.list("room")[0].owner,
        SessionProcessOwner::Agent("agent-1".into()),
        "the holding kernel keeps its own agent"
    );

    home.close_stdin("room", &id, input.len() as u64).unwrap();
    let (stdout, stderr, outcome) = collect(&home, &id).await;
    assert_eq!(stdout, input, "stdin reached the process byte for byte");
    assert_eq!(stderr, b"\xfferr");
    assert_eq!(
        outcome,
        ExecOutcome::Exited { code: 7 },
        "the target's exit code"
    );
    assert_eq!(home.list("room")[0].state, ExecState::Finished);
    assert!(!home.has_running_work());
    assert_eq!(
        second.open_command_ids(),
        vec![format!("place:{id}:open")],
        "one open, under an id derived from the launch"
    );
}

// The target ran the open and the answer was lost: the retry carries the same command id,
// the target's ledger answers with the first result, and nothing starts twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_answer_to_the_open_never_starts_a_second_process() {
    let second = SecondKernel::new(&[Fault::LoseReply]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "printf once; sleep 0.3", None);
    queued(&home, &id).await;
    home.place("room", &id, ORIGIN_NAME, second.placement())
        .unwrap();
    let (stdout, _, outcome) = collect(&home, &id).await;
    assert_eq!(stdout, b"once");
    assert_eq!(outcome, ExecOutcome::Exited { code: 0 });
    assert_eq!(second.execs().await.len(), 1, "one execution on the target");
    let opens = second.open_command_ids();
    assert_eq!(opens.len(), 2, "the open was sent twice");
    assert_eq!(opens[0], opens[1], "with the same command id");
}

// The open never reached the target twice: the start is ambiguous, so the launch ends
// failed with the outcome-unknown message and is never sent again.
#[tokio::test(flavor = "multi_thread")]
async fn an_ambiguous_open_ends_failed_and_is_never_replayed() {
    let second = SecondKernel::new(&[Fault::DropRequest, Fault::DropRequest, Fault::DropRequest]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "echo must-not-run", None);
    queued(&home, &id).await;
    home.place("room", &id, ORIGIN_NAME, second.placement())
        .unwrap();
    let (stdout, _, outcome) = collect(&home, &id).await;
    let ExecOutcome::Failed { message } = outcome else {
        panic!("expected a failed launch, got {outcome:?}");
    };
    assert!(
        message.contains("outcome is unknown") && message.contains("not started again"),
        "{message}"
    );
    assert!(stdout.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        second.open_command_ids().len(),
        2,
        "one retry, then never again"
    );
    assert!(second.execs().await.is_empty(), "nothing ran on the target");
    assert!(!home.has_running_work(), "and nothing ran at home");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_placed_launch_reaches_the_target_through_the_gateway() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let marker = home_dir.path().join("marker");
    let home = limited(0, u64::MAX / 2);
    let id = hold(
        &home,
        home_dir.path(),
        &format!("sleep 30; printf late > '{}'", marker.display()),
        None,
    );
    queued(&home, &id).await;
    home.place("room", &id, ORIGIN_NAME, second.placement())
        .unwrap();
    eventually(|| {
        (!second
            .router
            .runtime_state()
            .session_processes(&second.session_id)
            .is_empty())
        .then_some(())
    })
    .await;
    home.cancel("room", &id).unwrap();
    let (_, _, outcome) = collect(&home, &id).await;
    assert!(matches!(outcome, ExecOutcome::Failed { .. }), "{outcome:?}");
    let remote = second.execs().await;
    assert_eq!(remote.len(), 1);
    assert!(!marker.exists());
}

// The target labels a process with an agent of the origin kernel only under the origin
// name it verified for the gateway; a sender cannot claim another kernel's agent, and
// an ordinary Room agent id that does not exist there stays refused.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_verified_origin_name_can_label_a_process() {
    let second = SecondKernel::new(&[]);
    for claimed in ["kernel-z/agent-9", "agent-1", "kernel-a/"] {
        let refused = second
            .terminal(
                Some(claimed),
                RoomTerminalOperation::ExecOpen {
                    command: vec!["/bin/true".into()],
                },
            )
            .await
            .expect_err(claimed);
        assert!(!refused.code().is_empty(), "{claimed}");
    }
    assert!(second.execs().await.is_empty());
}

// The agent's own grant is the only authority: when its run has ended, nothing is sent.
#[tokio::test(flavor = "multi_thread")]
async fn an_ended_agent_grant_sends_nothing_and_the_launch_does_not_start() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "echo must-not-run", Some("agent-1"));
    queued(&home, &id).await;
    let caller = crate::runtime::state::reach::ReachCaller {
        id: "reach:ended-run".into(),
        run_id: "ended-run".into(),
        agent_id: "agent-1".into(),
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.into(),
        harness: "claude".into(),
    };
    let grant = OriginGrant::new(second.router.clone(), caller.origin(), "kernel-b");
    home.place(
        "room",
        &id,
        ORIGIN_NAME,
        Arc::new(GatewayExecPlacement::new(
            Arc::new(grant),
            second.session_id.clone(),
        )),
    )
    .unwrap();
    let (_, _, outcome) = collect(&home, &id).await;
    let ExecOutcome::Failed { message } = outcome else {
        panic!("expected a failed launch, got {outcome:?}");
    };
    assert!(
        message.contains("did not start") && message.contains("reach ended"),
        "{message}"
    );
    assert!(second.execs().await.is_empty());
    assert!(!home.has_running_work());
}

fn code_of(failure: &crate::error::DaemonError) -> &str {
    match failure {
        crate::error::DaemonError::MachineRequest { code, .. } => code,
        other => panic!("not a typed refusal: {other}"),
    }
}

// The `exec place` command: a Room is made on the target, then the held launch runs there.
fn placement<'a>(id: &'a str, directory: Option<&'a str>) -> Placement<'a> {
    Placement {
        session_id: "room",
        exec_id: id,
        directory,
        origin_kernel: ORIGIN_NAME,
    }
}

fn target_directory(second: &SecondKernel) -> String {
    second._workspace.path().display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owning_agent_places_its_held_launch_into_a_room_made_on_the_target() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "cat; exit 9", Some("agent-1"));
    queued(&home, &id).await;
    let directory = target_directory(&second);

    crate::room_exec::run_on_kernel(
        &home,
        placement(&id, Some(&directory)),
        &Requester::Agent("agent-1".into()),
        second.clone(),
    )
    .await
    .unwrap();

    home.write(
        "room",
        &id,
        0,
        &base64::engine::general_purpose::STANDARD.encode(b"hi"),
    )
    .unwrap();
    home.close_stdin("room", &id, 2).unwrap();
    let (stdout, _, outcome) = collect(&home, &id).await;
    assert_eq!(stdout, b"hi");
    assert_eq!(outcome, ExecOutcome::Exited { code: 9 });
    let sent = second.sent.lock().unwrap().clone();
    assert_eq!(
        sent.iter().filter(|id| id.ends_with(":room")).count(),
        1,
        "one Room made, under an id derived from the launch: {sent:?}"
    );
    assert_eq!(second.open_command_ids(), vec![format!("place:{id}:open")]);
    assert!(
        second.execs().await.is_empty(),
        "the launch runs in the Room made for it, not in the target's first Room"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_places_the_launch_of_an_agent() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "exit 3", Some("agent-1"));
    queued(&home, &id).await;
    let directory = target_directory(&second);
    crate::room_exec::run_on_kernel(
        &home,
        placement(&id, Some(&directory)),
        &Requester::Person,
        second.clone(),
    )
    .await
    .unwrap();
    let (_, _, outcome) = collect(&home, &id).await;
    assert_eq!(outcome, ExecOutcome::Exited { code: 3 });
}

#[tokio::test(flavor = "multi_thread")]
async fn another_agent_cannot_place_the_launch_and_nothing_is_sent() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let by_agent = hold(&home, home_dir.path(), "exit 3", Some("agent-1"));
    let by_person = hold(&home, home_dir.path(), "exit 3", None);
    queued(&home, &by_agent).await;
    queued(&home, &by_person).await;
    let directory = target_directory(&second);
    for id in [&by_agent, &by_person] {
        let refused = crate::room_exec::run_on_kernel(
            &home,
            placement(id, Some(&directory)),
            &Requester::Agent("agent-2".into()),
            second.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(code_of(&refused), "exec_not_yours");
    }
    assert!(
        second.sent.lock().unwrap().is_empty(),
        "no request left this kernel"
    );
    assert!(home
        .list("room")
        .iter()
        .all(|row| matches!(row.state, ExecState::Queued { .. })));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_launch_is_not_placed() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(u64::MAX / 2, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "sleep 30", Some("agent-1"));
    eventually(|| matches!(home.list("room")[0].state, ExecState::Running).then_some(())).await;
    let directory = target_directory(&second);
    let refused = crate::room_exec::run_on_kernel(
        &home,
        placement(&id, Some(&directory)),
        &Requester::Agent("agent-1".into()),
        second.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(code_of(&refused), "exec_not_queued");
    assert!(second.sent.lock().unwrap().is_empty());
    home.cancel("room", &id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_that_cannot_make_the_room_leaves_the_launch_held() {
    // The Room request is lost twice: the outcome is unknown, so the launch is not started
    // anywhere and stays held at home.
    let second = SecondKernel::new(&[Fault::DropRequest, Fault::DropRequest]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "exit 3", Some("agent-1"));
    queued(&home, &id).await;
    let directory = target_directory(&second);
    let failed = crate::room_exec::run_on_kernel(
        &home,
        placement(&id, Some(&directory)),
        &Requester::Agent("agent-1".into()),
        second.clone(),
    )
    .await
    .unwrap_err();
    assert_eq!(code_of(&failed), "kernel_outcome_unknown");
    assert!(second.open_command_ids().is_empty());
    assert!(matches!(
        home.list("room")[0].state,
        ExecState::Queued { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_kernel_is_a_typed_refusal_and_the_launch_stays_held() {
    struct Offline;
    impl GrantedSend for Offline {
        fn send<'a>(
            &'a self,
            _command_id: String,
            _request: &'a LocalDaemonRequest,
        ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
            Box::pin(async {
                Err(GatewayError::Offline {
                    name: "kernel-b".into(),
                })
            })
        }
    }
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "exit 3", None);
    queued(&home, &id).await;
    let failed = crate::room_exec::run_on_kernel(
        &home,
        Placement {
            session_id: "room",
            exec_id: &id,
            directory: Some("/"),
            origin_kernel: ORIGIN_NAME,
        },
        &Requester::Person,
        Arc::new(Offline),
    )
    .await
    .unwrap_err();
    assert_eq!(code_of(&failed), "kernel_offline");
    assert!(matches!(
        home.list("room")[0].state,
        ExecState::Queued { .. }
    ));
}

impl SecondKernel {
    async fn snapshot(&self) -> crate::local::WaitingRoomPublicSnapshot {
        let request = LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            crate::local::GetWaitingRoomPublicSnapshotRequest,
        );
        let answer = self
            .dispatch(format!("probe:{}", rand::random::<u64>()), &request)
            .await
            .unwrap();
        match serde_json::from_value(answer).unwrap() {
            LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => snapshot,
            other => panic!("not a snapshot: {other:?}"),
        }
    }
}

// Without a directory the Room is made where the target kernel itself would make one, not in
// the home Room's path, which may not exist there.
#[tokio::test(flavor = "multi_thread")]
async fn a_launch_placed_without_a_directory_gets_the_targets_own_default_room() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "exit 4", None);
    queued(&home, &id).await;
    let default = second
        .snapshot()
        .await
        .launch_target
        .expect("the target has a default Room directory");
    assert_ne!(default.workspace_id, home_dir.path().display().to_string());

    crate::room_exec::run_on_kernel(
        &home,
        placement(&id, None),
        &Requester::Person,
        second.clone(),
    )
    .await
    .unwrap();

    let (_, _, outcome) = collect(&home, &id).await;
    assert_eq!(outcome, ExecOutcome::Exited { code: 4 });
    let made = second
        .snapshot()
        .await
        .sessions
        .into_iter()
        .filter(|session| session.id != second.session_id)
        .collect::<Vec<_>>();
    assert_eq!(made.len(), 1, "one Room made for the launch");
    // The kernel may name the repository root as the Room's workspace; the checkout is the
    // directory the launch runs in.
    assert_eq!(made[0].worktree_id, default.worktree_id);
}

// `--directory` still decides, and then the target is not asked for a default.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_directory_is_used_and_the_target_default_is_not_read() {
    let second = SecondKernel::new(&[]);
    let home_dir = TempDir::new();
    let home = limited(0, u64::MAX / 2);
    let id = hold(&home, home_dir.path(), "exit 0", None);
    queued(&home, &id).await;
    let directory = target_directory(&second);
    crate::room_exec::run_on_kernel(
        &home,
        placement(&id, Some(&directory)),
        &Requester::Person,
        second.clone(),
    )
    .await
    .unwrap();
    let sent = second.sent.lock().unwrap().clone();
    assert!(sent.iter().all(|id| !id.ends_with(":target")), "{sent:?}");
}

#[test]
fn a_room_without_a_directory_needs_a_default_on_the_target() {
    use crate::room_exec::gateway_placement::room_location;
    let target = crate::local::WaitingRoomLaunchTarget {
        repository_state: None,
        git_available: false,
        workspace_id: "/srv/work".into(),
        worktree_id: "/srv/work/wt".into(),
        workspace_label: None,
        directory: None,
        worktree_label: None,
    };
    assert_eq!(
        room_location(None, Some(target.clone())).unwrap(),
        ("/srv/work".to_string(), "/srv/work/wt".to_string())
    );
    assert_eq!(
        room_location(Some("/given"), Some(target)).unwrap(),
        ("/given".to_string(), "/given".to_string())
    );
    assert_eq!(
        room_location(Some("/given"), None).unwrap(),
        ("/given".to_string(), "/given".to_string())
    );
    let refused = room_location(None, None).unwrap_err();
    assert_eq!(code_of(&refused), "exec_target_no_directory");
    assert!(refused.to_string().contains("--directory"), "{refused}");
}
