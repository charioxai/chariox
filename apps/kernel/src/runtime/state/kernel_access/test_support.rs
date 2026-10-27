use super::*;

mod approval;
mod capability_room_authority;
mod control_authority;
mod cover;
mod credential_authority;
mod gateway_authority;
mod inherited_grants;
mod inner_authority;
mod input_control_authority;
mod meta_authority;
mod native_launch_authority;
mod session_authority;
pub(in crate::runtime::state) mod worker_spy;
mod workflow_authority;

impl KernelRuntimeState {
    pub(crate) fn insert_access_grant_for_test(&self, session_id: &str) -> String {
        let session = self.owned.session_store.get_session(session_id).unwrap();
        let holder = process::inspect(std::process::id()).unwrap().0;
        let id = format!("queue-access-{:016x}", rand::random::<u64>());
        let grant = Grant {
            covered: Vec::new(),
            summary: KernelAccessGrant {
                covered: Vec::new(),
                grant_id: id.clone(),
                owner_user_id: session.owner_user_id().into(),
                holder_pid: holder.pid,
                holder_executable: holder.executable.clone(),
                holder_harness: None,
                term: KernelAccessTerm::Minutes(30),
                expires_at_ms: Some(crate::session::unix_epoch_ms() + 1_800_000),
            },
            holder,
            clock: Some(GrantClock::new(30, 5)),
        };
        self.owned
            .kernel_access
            .lock()
            .unwrap()
            .grants
            .insert(id.clone(), grant);
        id
    }

    pub(crate) fn use_kernel_ancestor_holder_for_test(&self) -> String {
        let (_, parent) = process::inspect(std::process::id()).unwrap();
        let mut state = self.owned.kernel_access.lock().unwrap();
        let mut fixture = state.grants.values().next().unwrap().clone();
        fixture.holder = process::inspect(parent).unwrap().0;
        fixture.summary.grant_id = "kernel-ancestor-fixture".into();
        state
            .grants
            .insert(fixture.summary.grant_id.clone(), fixture);
        "kernel-ancestor-fixture".into()
    }

    pub(crate) fn restore_access_holder_for_test(&self, id: String) {
        self.owned.kernel_access.lock().unwrap().grants.remove(&id);
    }

    pub(crate) async fn control_access_for_test(&self, action: &str, vault: &std::path::Path) {
        if self.control_credential_access_for_test(action, vault).await {
            return;
        }
        if action == "guest" {
            assert!(self.passkey_prompts_for("guest").is_empty());
            let prompt = self
                .passkey_prompts_for(crate::session::DEFAULT_LOCAL_USER_ID)
                .into_iter()
                .find(|p| p.kind == PasskeyPromptKind::AccessGrant)
                .unwrap();
            assert!(self
                .answer_terminal_runtime_interaction(
                    &prompt.session_id,
                    &prompt.interaction_id,
                    "approve",
                    None,
                    Some("guest"),
                    Some(&ApprovalPasskey::new("Access TEST Passkey")),
                    None,
                    Some(KernelConnectionClass::Terminal)
                )
                .await
                .is_err());
            return;
        }
        if action == "rotate" {
            self.change_vault_passphrase(
                crate::session::DEFAULT_LOCAL_USER_ID,
                vault,
                zeroize::Zeroizing::new("Access TEST Passkey".into()),
                zeroize::Zeroizing::new("Rotated TEST Passkey".into()),
            )
            .await
            .unwrap();
            return;
        }
        if action == "sudo-timeout" {
            let prompt = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(prompt) = self
                        .passkey_prompts_for("local")
                        .into_iter()
                        .find(|p| p.kind == PasskeyPromptKind::Sudo)
                    {
                        break prompt;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            self.owned
                .timeout_runtime_interaction(&prompt.session_id, &prompt.interaction_id)
                .unwrap();
            return;
        }
        if action == "timeout" {
            let prompts = self
                .owned
                .pending_interactions
                .write()
                .iter()
                .filter(|(_, pending)| {
                    pending
                        .passkey_prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt.kind == PasskeyPromptKind::AccessGrant)
                })
                .map(|(id, p)| (id.clone(), p.session_id.clone()))
                .collect::<Vec<_>>();
            for (id, session) in prompts {
                self.owned
                    .timeout_runtime_interaction(&session, &id)
                    .unwrap();
            }
            return;
        }
        let mut state = self.owned.kernel_access.lock().unwrap();
        for grant in state.grants.values_mut() {
            match action {
                "expire" => {
                    if let Some(clock) = grant.clock.as_mut() {
                        clock.deadline = Instant::now() - Duration::from_millis(1);
                    }
                }
                // The deadline arrives by itself; nothing sweeps on its behalf.
                "expire-soon" => {
                    if let Some(clock) = grant.clock.as_mut() {
                        clock.deadline = Instant::now() + Duration::from_millis(1500);
                    }
                }
                "notice" => {
                    if let Some(clock) = grant.clock.as_mut() {
                        clock.notice = Instant::now() - Duration::from_millis(1);
                    }
                }
                "reuse" => grant.holder.start = grant.holder.start.wrapping_add(1),
                _ => panic!("unknown test control"),
            }
        }
        drop(state);
        self.pump_kernel_access();
    }
}

// MP-08 / MP-10 / MP-11: granting access needs no session, and the shared
// owner/timeout machinery still protects the kernel-wide board.
#[tokio::test]
async fn kernel_access_popup_works_before_first_session_and_expires_on_shared_board() {
    for owner in ["local", "cloud-local-owner"] {
        let mut config = crate::config::DaemonConfig::for_tests();
        if owner != "local" {
            config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
                user_id: owner.into(),
                ..Default::default()
            });
        }
        let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            32,
        );
        let state = router.runtime_state();
        assert!(state
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .is_empty());
        let scope = crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE;
        let decision = |id: &str| {
            RuntimeInteraction::for_kernel_operation(
                id,
                format!("access-grant:{id}"),
                "Local kernel access",
                "whole LOCAL kernel",
                vec![
                    RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                        .requiring_passkey(),
                ],
            )
            .with_timeout_sec(60)
        };
        let rx = state
            .create_kernel_operation_interaction(scope, owner, decision("no-session-access"))
            .await
            .unwrap();
        assert_eq!(state.passkey_prompts_for(owner).len(), 1);
        assert!(state.passkey_prompts_for("guest").is_empty());
        assert!(state
            .answer_terminal_runtime_interaction(
                scope,
                "no-session-access",
                "refuse",
                None,
                Some("guest"),
                None,
                None,
                Some(KernelConnectionClass::Terminal)
            )
            .await
            .is_err());
        state
            .answer_terminal_runtime_interaction(
                scope,
                "no-session-access",
                "refuse",
                None,
                Some(owner),
                None,
                None,
                Some(KernelConnectionClass::Terminal),
            )
            .await
            .unwrap();
        assert_eq!(rx.await.unwrap().choice_id.as_deref(), Some("refuse"));
        assert!(state.passkey_prompts_for(owner).is_empty());
        let rx = state
            .create_kernel_operation_interaction(scope, owner, decision("no-session-expiry"))
            .await
            .unwrap();
        state
            .owned
            .timeout_runtime_interaction(scope, "no-session-expiry")
            .unwrap();
        assert_eq!(rx.await.unwrap().status, "timed_out");
        assert!(state.passkey_prompts_for(owner).is_empty());
    }
}

// EA2-01: a grant that lasts until its holder exits has no deadline, however
// much time passes, while a timed grant of the same holder ends at its own.
#[tokio::test]
async fn until_exit_grant_outlives_every_deadline_but_not_its_holder() {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state();
    let live_holder = process::inspect(std::process::id()).unwrap().0;
    let grant = |id: &str, term, clock, holder: ProcessIdentity| Grant {
        covered: Vec::new(),
        summary: KernelAccessGrant {
            covered: Vec::new(),
            grant_id: id.into(),
            owner_user_id: "local".into(),
            holder_pid: holder.pid,
            holder_executable: holder.executable.clone(),
            holder_harness: None,
            term,
            expires_at_ms: None,
        },
        holder,
        clock,
    };
    let mut reused = live_holder.clone();
    reused.start = reused.start.wrapping_add(1);
    {
        let mut access = state.owned.kernel_access.lock().unwrap();
        access.grants.insert(
            "timed".into(),
            grant(
                "timed",
                KernelAccessTerm::Minutes(480),
                Some(GrantClock::new(480, 5)),
                live_holder.clone(),
            ),
        );
        access.grants.insert(
            "until-exit".into(),
            grant(
                "until-exit",
                KernelAccessTerm::UntilHolderExits,
                None,
                live_holder.clone(),
            ),
        );
        access.grants.insert(
            "until-exit-pid-reused".into(),
            grant(
                "until-exit-pid-reused",
                KernelAccessTerm::UntilHolderExits,
                None,
                reused,
            ),
        );
    }
    state.sweep_kernel_access_at(Instant::now() + Duration::from_secs(48 * 3600));
    let mut live = state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    live.sort();
    assert_eq!(live, ["until-exit"]);
    // Neither grant may ask the owner to extend it: only a clock can.
    state.sweep_kernel_access_at(Instant::now() + Duration::from_secs(96 * 3600));
    assert!(state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .contains_key("until-exit"));
}

// A grant that lasts until its holder exits goes through a kernel restart with
// the same id and the same event inbox while the holder lives; a timed grant
// and a grant whose holder is gone do not.
#[tokio::test]
async fn until_exit_grant_survives_a_restart_while_its_holder_lives() {
    let root = crate::test_support::TestWorktree::new("access-restart");
    let kernel = || {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.user_config_path = root.path().join("config.toml");
        let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            32,
        )
    };
    let live_holder = process::inspect(std::process::id()).unwrap().0;
    let mut child = std::process::Command::new("sleep")
        .arg("600")
        .spawn()
        .unwrap();
    let doomed_holder = process::inspect(child.id()).unwrap().0;
    let summary = |id: &str, term, holder: &ProcessIdentity| KernelAccessGrant {
        covered: Vec::new(),
        grant_id: id.into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term,
        expires_at_ms: None,
    };
    let before = {
        let router = kernel();
        let state = router.runtime_state();
        let grants = [
            (
                "kept",
                KernelAccessTerm::UntilHolderExits,
                None,
                &live_holder,
            ),
            (
                "timed",
                KernelAccessTerm::Minutes(30),
                Some(GrantClock::new(30, 5)),
                &live_holder,
            ),
            (
                "doomed",
                KernelAccessTerm::UntilHolderExits,
                None,
                &doomed_holder,
            ),
        ];
        for (id, term, clock, holder) in grants {
            let mut access = state.owned.kernel_access.lock().unwrap();
            state
                .admit_grant(
                    &mut access,
                    id.into(),
                    summary(id, term, holder),
                    holder.clone(),
                    clock,
                )
                .unwrap();
        }
        let inbox = state.outside_subscription_response("kept").unwrap();
        // Stopping releases a waiting holder with an empty answer, not with
        // the end of its subscription.
        state.end_kernel_access_for_shutdown().unwrap();
        let released = state
            .outside_wait(
                "kept",
                &WaitEventsRequest {
                    since: None,
                    timeout_ms: 5_000,
                    limit: 10,
                },
                Some("kept"),
                None,
            )
            .await
            .unwrap();
        assert!(
            matches!(
                released,
                LocalDaemonResponse::EventsListed {
                    timed_out: true,
                    ..
                }
            ),
            "{released:?}"
        );
        inbox
    };
    child.kill().unwrap();
    child.wait().unwrap();
    let router = kernel();
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    let mut live = state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    live.sort();
    assert_eq!(live, ["kept"]);
    assert_eq!(state.outside_subscription_response("kept").unwrap(), before);
    assert!(state.outside_ended_response("doomed").unwrap().is_some());
    assert!(state.outside_ended_response("timed").unwrap().is_some());
}

/// An until-exit grant whose revocation could not be written: the grant is
/// gone from memory, the fault is still on, and its holder (this process) lives.
fn failed_revocation(
    root: &crate::test_support::TestWorktree,
) -> (
    crate::runtime::router::CommandRouter,
    std::path::PathBuf,
    crate::durable_state::agent_lifecycle::faults::TestFaultDir,
) {
    let faults = root.path().join("faults");
    std::fs::create_dir(&faults).unwrap();
    let guard = crate::durable_state::agent_lifecycle::faults::TestFaultDir::set(&faults);
    let router = reopen_kernel(root);
    let state = router.runtime_state();
    let holder = process::inspect(std::process::id()).unwrap().0;
    let summary = KernelAccessGrant {
        covered: Vec::new(),
        grant_id: "kept".into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term: KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
    };
    {
        let mut access = state.owned.kernel_access.lock().unwrap();
        state
            .admit_grant(&mut access, "kept".into(), summary, holder, None)
            .unwrap();
    }
    std::fs::write(faults.join("writer-fail"), "").unwrap();
    assert!(state
        .revoke_kernel_access(None, Some("kept"), "revoked_by_owner")
        .is_err());
    assert!(state.owned.kernel_access.lock().unwrap().grants.is_empty());
    (router, faults, guard)
}

fn reopen_kernel(
    root: &crate::test_support::TestWorktree,
) -> crate::runtime::router::CommandRouter {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.user_config_path = root.path().join("config.toml");
    let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
}

// A revocation the kernel could not finish stays revoked: the next start, with
// the holder still alive, must not bring the grant back.
#[tokio::test]
async fn a_revocation_that_failed_to_write_is_not_undone_by_a_restart() {
    let root = crate::test_support::TestWorktree::new("access-revoke-restart");
    let (router, faults, _guard) = failed_revocation(&root);
    drop(router);
    std::fs::remove_file(faults.join("writer-fail")).unwrap();
    let router = reopen_kernel(&root);
    assert_eq!(
        router
            .runtime_state()
            .restore_kernel_access_grants()
            .unwrap(),
        0
    );
}

// The inbox of a revoked grant is ended as soon as the store takes writes again.
#[tokio::test]
async fn a_revoked_grants_inbox_is_ended_once_the_store_takes_writes_again() {
    let root = crate::test_support::TestWorktree::new("access-revoke-retry");
    let (router, faults, _guard) = failed_revocation(&root);
    let state = router.runtime_state();
    assert!(state.outside_ended_response("kept").unwrap().is_none());
    std::fs::remove_file(faults.join("writer-fail")).unwrap();
    state.sweep_kernel_access();
    assert!(state.outside_ended_response("kept").unwrap().is_some());
}

/// Makes the audit log refuse revocation records, as a full disk refuses them.
fn refuse_revocation_records(root: &crate::test_support::TestWorktree, refuse: bool) {
    let connection = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
    connection
        .execute_batch(if refuse {
            "CREATE TRIGGER refuse_revocation BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'kernel_access.grant' AND NEW.payload_json LIKE '%\"revoked\"%'
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;"
        } else {
            "DROP TRIGGER refuse_revocation;"
        })
        .unwrap();
}

// The audit append itself fails: the revocation is kept, a retried revoke
// writes it once storage recovers, and a restart with the holder alive does
// not bring the until-exit grant back.
#[tokio::test]
async fn a_revocation_the_audit_could_not_append_is_written_by_the_retry() {
    let root = crate::test_support::TestWorktree::new("access-revoke-append");
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let holder = process::inspect(std::process::id()).unwrap().0;
    let summary = KernelAccessGrant {
        covered: Vec::new(),
        grant_id: "kept".into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term: KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
    };
    {
        let mut access = state.owned.kernel_access.lock().unwrap();
        state
            .admit_grant(&mut access, "kept".into(), summary, holder, None)
            .unwrap();
    }
    refuse_revocation_records(&root, true);
    assert!(state
        .revoke_kernel_access(None, Some("kept"), "revoked_by_owner")
        .is_err());
    assert!(state.owned.kernel_access.lock().unwrap().grants.is_empty());
    // Still refused: the retry fails too and the revocation stays owed.
    assert!(state
        .revoke_kernel_access(None, Some("kept"), "revoked_by_owner")
        .is_err());
    refuse_revocation_records(&root, false);
    assert_eq!(
        state
            .revoke_kernel_access(None, Some("kept"), "revoked_by_owner")
            .unwrap(),
        1
    );
    assert!(state.outside_ended_response("kept").unwrap().is_some());
    drop(state);
    drop(router);
    let router = reopen_kernel(&root);
    assert_eq!(
        router
            .runtime_state()
            .restore_kernel_access_grants()
            .unwrap(),
        0
    );
}

// The sweep also finishes a revocation whose append failed, with no new revoke.
#[tokio::test]
async fn the_sweep_writes_a_revocation_the_audit_could_not_append() {
    let root = crate::test_support::TestWorktree::new("access-revoke-append-sweep");
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let holder = process::inspect(std::process::id()).unwrap().0;
    let summary = KernelAccessGrant {
        covered: Vec::new(),
        grant_id: "kept".into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term: KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
    };
    {
        let mut access = state.owned.kernel_access.lock().unwrap();
        state
            .admit_grant(&mut access, "kept".into(), summary, holder, None)
            .unwrap();
    }
    refuse_revocation_records(&root, true);
    assert!(state
        .revoke_kernel_access(None, Some("kept"), "revoked_by_owner")
        .is_err());
    refuse_revocation_records(&root, false);
    state.sweep_kernel_access();
    drop(state);
    drop(router);
    let router = reopen_kernel(&root);
    assert_eq!(
        router
            .runtime_state()
            .restore_kernel_access_grants()
            .unwrap(),
        0
    );
}

// A grant holder chooses among the choices a routine provider permission
// carries, `allow_session` included; a critical decision stays terminal-only.
#[tokio::test]
async fn grant_holder_can_choose_allow_session_on_a_routine_permission() {
    use crate::session::{
        RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind,
        RuntimeInteractionLevel,
    };
    let workspace = crate::test_support::TestWorktree::new("allow-session-grant");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            session.id(),
            "dev-stub",
        ))
        .unwrap();
    let state =
        crate::runtime::state::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
            app,
        );
    let grant = state.insert_access_grant_for_test(session.id());
    let permission = |id: &str, agent_id: &str, level| {
        RuntimeInteraction::new(
            id,
            agent_id,
            RuntimeInteractionKind::Permission,
            level,
            Some("Approve Tool?".to_string()),
            "A tool call",
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                RuntimeInteractionChoice::new(
                    "allow_session",
                    "Allow for this session",
                    "allow",
                    None,
                ),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        )
    };
    let answer = |id: &str| {
        LocalDaemonRequest::RespondToInteraction(
            serde_json::from_value(serde_json::json!({
                "session_id": session.id(), "interaction_id": id, "choice_id": "allow_session",
            }))
            .unwrap(),
        )
    };
    for (id, agent_id, level) in [
        (
            "routine-permission",
            agent.id(),
            RuntimeInteractionLevel::Warning,
        ),
        (
            "critical-permission",
            second.id(),
            RuntimeInteractionLevel::Critical,
        ),
    ] {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state
            .owned
            .register_runtime_interaction(
                session.id(),
                permission(id, agent_id, level),
                tx,
                None,
                None,
                None,
            )
            .unwrap();
    }
    state
        .authorize_external_request(&grant, &answer("routine-permission"))
        .expect("a holder may choose allow_session on a routine permission");
    let refused = state
        .authorize_external_request(&grant, &answer("critical-permission"))
        .expect_err("a critical decision stays terminal-only");
    assert_eq!(
        crate::runtime::kernel_access::Refusal::of(&refused),
        crate::runtime::kernel_access::Refusal::HumanOnly
    );
}

// A decision raised while nobody follows its agent (a kernel restart gap) is
// told to the holder that follows afterwards, once, and can then be answered.
#[tokio::test]
async fn a_decision_open_before_the_holder_subscribes_is_delivered_on_subscribe() {
    use crate::session::{
        RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind,
        RuntimeInteractionLevel,
    };
    let workspace = crate::test_support::TestWorktree::new("open-decision-replay");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let other = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            session.id(),
            "dev-stub",
        ))
        .unwrap();
    let state =
        crate::runtime::state::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
            app,
        );
    let grant = state.insert_access_grant_for_test(session.id());
    let _responders: Vec<_> = [
        ("followed-permission", agent.id()),
        ("other-permission", other.id()),
    ]
    .into_iter()
    .map(|(id, agent_id)| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .owned
            .register_runtime_interaction(
                session.id(),
                RuntimeInteraction::new(
                    id,
                    agent_id,
                    RuntimeInteractionKind::Permission,
                    RuntimeInteractionLevel::Warning,
                    Some("Approve Tool?".to_string()),
                    "A tool call",
                    vec![
                        RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", None),
                        RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    ],
                    None,
                    Some(300),
                    Some("deny".to_string()),
                ),
                tx,
                None,
                None,
                None,
            )
            .unwrap();
        rx
    })
    .collect();
    let target = EventTarget {
        kernel: state.owned.config_projection.snapshot().daemon_id,
        session_id: session.id().into(),
        agent_id: Some(agent.id().into()),
    };
    let decisions = |state: &KernelRuntimeState| {
        let batch = state
            .outside_store()
            .outside_list(&grant, None, 100)
            .unwrap()
            .unwrap();
        batch
            .events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.value,
                    OutsideEventValue::Interaction {
                        state: EventInteractionState::Pending,
                        ..
                    }
                )
            })
            .map(|event| (event.agent_id, event.refs.interaction_id))
            .collect::<Vec<_>>()
    };
    state
        .subscribe_outside_part(&grant, true, vec![target.clone()])
        .unwrap();
    assert_eq!(
        decisions(&state),
        [(
            Some(agent.id().to_string()),
            Some("followed-permission".to_string())
        )],
        "only the followed agent's open decision is told"
    );
    // Following the same target again must not tell it a second time.
    state
        .subscribe_outside_part(&grant, true, vec![target])
        .unwrap();
    assert_eq!(decisions(&state).len(), 1);
    let answer = LocalDaemonRequest::RespondToInteraction(
        serde_json::from_value(serde_json::json!({
            "session_id": session.id(),
            "interaction_id": "followed-permission",
            "choice_id": "allow_once",
        }))
        .unwrap(),
    );
    state
        .authorize_external_request(&grant, &answer)
        .expect("the holder can answer the decision it was told about");
}
