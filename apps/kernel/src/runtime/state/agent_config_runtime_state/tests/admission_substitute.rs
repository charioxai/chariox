//! A turn whose primary account is refused at admission (usage exhausted or
//! not signed in) runs on the first usable substitute instead of waiting for
//! the primary, and the Room hears about each state change once.

use super::*;
use crate::account_profile::{
    ProviderAccountProfile, ProviderAccountProfileRegistry, ProviderAccountUsageSnapshot,
};
use crate::provider::ProviderRunState;

const PRIMARY_MODEL: &str = "gpt-test";
const SUBSTITUTE_A: &str = "substitute-a";
const SUBSTITUTE_B: &str = "substitute-b";
const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

struct RefusedPrimary {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    _worktree: crate::test_support::TestWorktree,
}

fn far_future_ms() -> u64 {
    crate::session::unix_epoch_ms() + 6 * 24 * 60 * 60 * 1000
}

/// A Codex agent whose `default` account is exhausted, with `substitutes`
/// (dev-stub models) configured in order.
async fn refused_primary(substitutes: &[&str]) -> RefusedPrimary {
    let fixture = healthy_primary(substitutes).await;
    exhaust_primary_account(&fixture);
    fixture
}

fn exhaust_primary_account(fixture: &RefusedPrimary) {
    exhaust_provider_account(
        &fixture.runtime.owned.provider_account_profiles,
        OWNER,
        "codex",
        "default",
        far_future_ms(),
    )
    .unwrap();
}

/// The same agent while its `default` account is still usable.
async fn healthy_primary(substitutes: &[&str]) -> RefusedPrimary {
    let worktree = crate::test_support::TestWorktree::new("admission-substitute");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            worktree.path().to_string_lossy(),
            worktree.path().to_string_lossy(),
        ))
        .expect("session should be created");
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let attachment_id = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "substitute-test",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap()
        .id()
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent_id,
            "codex",
            Some(PRIMARY_MODEL.to_string()),
            None,
            Some("default".to_string()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    for model in substitutes {
        runtime
            .owned
            .agent_store
            .add_agent_substitute(
                &agent_id,
                crate::agent::AgentSubstituteProfile::new("dev-stub", *model, None),
            )
            .unwrap();
    }
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        OWNER,
        "codex",
        "default",
    )
    .unwrap();
    RefusedPrimary {
        runtime,
        session_id,
        agent_id,
        attachment_id,
        _worktree: worktree,
    }
}

impl RefusedPrimary {
    fn prepared(&self, id: &str) -> crate::app::KernelPreparedPromptSubmission {
        crate::app::KernelPreparedPromptSubmission {
            session_id: self.session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                id,
                &self.attachment_id,
                &self.agent_id,
                "review this change",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        }
    }

    fn live_run(&self) -> Option<crate::provider::RuntimeProviderRun> {
        self.runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.session_id, &self.agent_id)
            .filter(|run| run.state() != ProviderRunState::Ended)
    }

    async fn wait_for_live_run(&self) -> crate::provider::RuntimeProviderRun {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(run) = self.live_run() {
                    return run;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the turn must reach a live provider run within one retry")
    }
}

// S1: a plain agent turn on an exhausted primary runs on the first substitute.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exhausted_primary_runs_a_plain_turn_on_the_first_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = refused_primary(&[SUBSTITUTE_A, SUBSTITUTE_B]).await;

    fixture
        .runtime
        .submit_prepared_prompt(fixture.prepared("exhausted-primary"))
        .await
        .expect("an exhausted primary with a usable substitute must admit the turn");

    let run = fixture.wait_for_live_run().await;
    assert_eq!(run.model(), SUBSTITUTE_A);
    let turn = run.turn_substitute().expect("the run is a substitute's");
    assert_eq!(turn.tried.len(), 1);
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let active = fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &fixture.agent_id)
        .unwrap();
    assert_eq!(
        turn.prompt_id,
        active.id(),
        "the substitute serves the exact admitted turn"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .list_runs()
            .iter()
            .filter(
                |run| run.agent_instance_id() == Some(fixture.agent_id.as_str())
                    && run.turn_substitute().is_some()
            )
            .count(),
        1,
        "canonicalizing the admitted prompt must not launch a second substitute"
    );
    let agent = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent_id)
        .unwrap();
    assert_eq!(
        agent.provider(),
        "codex",
        "the configured profile never changes"
    );
    assert_eq!(agent.model(), Some(PRIMARY_MODEL));
}

// S1: a substitute that is itself exhausted is skipped for the next one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exhausted_substitute_is_skipped_at_admission() {
    crate::test_support::isolated_env_test!();
    let fixture = refused_primary(&[]).await;
    for (provider, model) in [("codex", "gpt-other"), ("dev-stub", SUBSTITUTE_B)] {
        fixture
            .runtime
            .owned
            .agent_store
            .add_agent_substitute(
                &fixture.agent_id,
                crate::agent::AgentSubstituteProfile::new(provider, model, None),
            )
            .unwrap();
    }
    // The codex substitute shares the exhausted `default` account.
    fixture
        .runtime
        .submit_prepared_prompt(fixture.prepared("skip-exhausted-substitute"))
        .await
        .expect("a later usable substitute must admit the turn");

    let run = fixture.wait_for_live_run().await;
    assert_eq!(run.model(), SUBSTITUTE_B);
}

// An idle primary process that is still running between turns holds no turn: when its account
// is refused after the last one, the next prompt goes to the substitute, not to that process.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_idle_primary_whose_account_is_refused_hands_the_next_turn_to_the_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = refused_primary(&[SUBSTITUTE_A]).await;
    // The process was started while the account was usable and is idle now.
    let request = crate::provider::LaunchProviderRequest::new(
        &fixture.session_id,
        "codex",
        "codex",
        "default",
        PRIMARY_MODEL,
    )
    .with_agent_id(&fixture.agent_id);
    let mut primary = crate::provider::RuntimeProviderRun::new(
        "idle-primary-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    primary.mark_running();
    fixture
        .runtime
        .with_app_side_effect(move |app| {
            app.providers_mut().insert_run_for_test(primary.clone());
            Ok::<(), crate::error::DaemonError>(())
        })
        .await
        .unwrap();
    assert_eq!(fixture.wait_for_live_run().await.id(), "idle-primary-run");

    let mut submission = fixture
        .runtime
        .submit_prepared_prompt(fixture.prepared("after-the-refusal"))
        .await
        .expect("a refused idle primary with a usable substitute must admit the turn");
    // The caller sends the started prompt through the dispatcher.
    let dispatch = submission
        .dispatch
        .take()
        .expect("the prompt starts on the idle process");
    fixture
        .runtime
        .enqueue_prompt_dispatch_after_liveness(&dispatch, &fixture.runtime.owned)
        .await
        .expect("the dispatcher accepts the prompt");

    let run = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(run) = fixture
                .live_run()
                .filter(|run| run.turn_substitute().is_some())
            {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the turn runs on a substitute's process");
    assert_eq!(run.model(), SUBSTITUTE_A);
    assert_ne!(run.id(), "idle-primary-run");
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run("idle-primary-run")
            .map(|run| run.state())
            .unwrap(),
        ProviderRunState::Ended,
        "the refused process is retired"
    );
}

impl RefusedPrimary {
    /// A live primary process of `adapter_key` that serves turn A, with turn B queued behind it.
    /// Nothing is dispatched to the process: the turns are only admitted.
    async fn running_primary_with_a_queued_successor(&self, adapter_key: &str) {
        let request = crate::provider::LaunchProviderRequest::new(
            &self.session_id,
            adapter_key,
            "codex",
            "default",
            PRIMARY_MODEL,
        )
        .with_agent_id(&self.agent_id);
        let mut primary = crate::provider::RuntimeProviderRun::new(
            "serving-primary-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-codex-runtime".to_string()),
            },
        );
        primary.mark_running();
        self.runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(primary);
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap();
        for id in ["turn-a", "turn-b"] {
            self.runtime
                .owned
                .prompt_state_owner
                .submit_prepared_prompt_with_queue_policy(
                    &session,
                    self.prepared(id).prompt,
                    false,
                    true,
                )
                .unwrap();
        }
        let (active, queued) = self
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &self.agent_id);
        self.runtime
            .owned
            .mirror_prompt_owner_agent_state(&self.session_id, &self.agent_id, active, queued)
            .unwrap();
    }

    fn queued_head(&self) -> crate::session::PromptQueueItem {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap();
        self.runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, &self.agent_id)
            .expect("turn B is queued")
    }

    /// The dispatch that starts turn B once turn A has settled, run to its end.
    async fn dispatch_then_expect_the_substitute(
        &self,
        dispatch: crate::runtime::state::owned::NextPromptDispatch,
    ) {
        let crate::runtime::state::owned::NextPromptDispatch::Submit(dispatch) = dispatch else {
            panic!("the queued turn must be submitted, not settled as a failed dispatch");
        };
        self.runtime
            .enqueue_prompt_dispatch_after_liveness(&dispatch, &self.runtime.owned)
            .await
            .expect("the queued turn is dispatched");
        let run = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(run) = self
                    .live_run()
                    .filter(|run| run.turn_substitute().is_some())
                {
                    return run;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the queued turn runs on a substitute's process");
        assert_eq!(run.model(), SUBSTITUTE_A);
        assert_eq!(
            self.runtime
                .owned
                .provider_store
                .get_run("serving-primary-run")
                .map(|run| run.state())
                .unwrap(),
            ProviderRunState::Ended,
            "the refused process is retired"
        );
    }
}

// The primary serves turn A with turn B queued; its account is refused before A settles. B is
// not started on the refused process, whichever way the process takes its prompts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_successor_of_a_refused_structured_primary_runs_on_the_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = healthy_primary(&[SUBSTITUTE_A]).await;
    fixture
        .running_primary_with_a_queued_successor("codex")
        .await;
    exhaust_primary_account(&fixture);

    let completion = fixture
        .runtime
        .owned
        .complete_local_prompt_with_queued_advance_if_matches(
            &fixture.session_id,
            &fixture.agent_id,
            Some("serving-primary-run"),
            &fixture.queued_head(),
            Some("turn-a"),
        )
        .unwrap()
        .expect("turn A settles");

    assert!(
        completion.completion.started_next.is_some(),
        "turn B is promoted"
    );
    let dispatch = completion
        .dispatch
        .expect("turn B goes through the dispatcher, not straight to the refused process");
    fixture.dispatch_then_expect_the_substitute(dispatch).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_successor_of_a_refused_primary_runs_on_the_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = healthy_primary(&[SUBSTITUTE_A]).await;
    fixture
        .running_primary_with_a_queued_successor("dev-stub")
        .await;
    exhaust_primary_account(&fixture);

    let completion = fixture
        .runtime
        .owned
        .complete_local_prompt_with_queued_advance_if_matches(
            &fixture.session_id,
            &fixture.agent_id,
            Some("serving-primary-run"),
            &fixture.queued_head(),
            Some("turn-a"),
        )
        .unwrap()
        .expect("turn A settles");

    let dispatch = completion.dispatch.expect("turn B is dispatched");
    fixture.dispatch_then_expect_the_substitute(dispatch).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_successor_of_a_cancelled_turn_on_a_refused_primary_runs_on_the_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = healthy_primary(&[SUBSTITUTE_A]).await;
    fixture
        .running_primary_with_a_queued_successor("dev-stub")
        .await;
    exhaust_primary_account(&fixture);

    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    fixture
        .runtime
        .owned
        .prompt_state_owner
        .begin_cancelling_prompt_if_matches(&session, &fixture.agent_id, Some("turn-a"))
        .expect("turn A is cancelling");
    let cancellation = fixture
        .runtime
        .owned
        .finalize_local_prompt_cancellation_with_queued_advance(
            &fixture.session_id,
            &fixture.agent_id,
            "turn-a",
            Some("serving-primary-run"),
        )
        .unwrap();

    let dispatch = cancellation.dispatch.expect("turn B is dispatched");
    fixture.dispatch_then_expect_the_substitute(dispatch).await;
}

/// The OpenCode `default` account of the test owner, signed in nowhere.
fn opencode_account_without_login(fixture: &RefusedPrimary) {
    let registry = &fixture.runtime.owned.provider_account_profiles;
    if registry
        .find(OWNER, "opencode", "default")
        .unwrap()
        .is_none()
    {
        let created = registry
            .create_managed(OWNER, "opencode", "Free models")
            .unwrap();
        registry
            .set_default(OWNER, "opencode", &created.profile_id)
            .unwrap();
    }
}

// OpenCode's free models run with no account, as in normal admission: such a substitute is
// usable without a login, and a Go model is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_free_opencode_substitute_runs_without_a_login_and_a_go_one_does_not() {
    crate::test_support::isolated_env_test!();
    let fixture = refused_primary(&[]).await;
    opencode_account_without_login(&fixture);
    let choose = |model: &str| {
        let candidate = crate::agent::AgentSubstituteProfile::new("opencode", model, None);
        crate::substitute_admission::unusable_reason(
            &fixture.runtime.owned.provider_account_profiles,
            &fixture.runtime.owned.config_projection.snapshot(),
            OWNER,
            &candidate,
        )
        .unwrap()
    };

    assert_eq!(
        choose("opencode/free-model"),
        None,
        "a free model needs no login"
    );
    assert!(
        choose("opencode-go/paid-model").is_some(),
        "a Go model needs its credential"
    );
}

// The Go preflight of a plain submission reaches the substitute: an unauthenticated Go primary
// with a usable substitute runs the turn there instead of refusing it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unauthenticated_go_primary_runs_a_plain_turn_on_the_substitute() {
    crate::test_support::isolated_env_test!();
    let fixture = healthy_primary(&[SUBSTITUTE_A]).await;
    opencode_account_without_login(&fixture);
    fixture
        .runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &fixture.agent_id,
            "opencode",
            Some("opencode-go/primary-model".to_string()),
            None,
            Some("default".to_string()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();

    fixture
        .runtime
        .submit_prepared_prompt(fixture.prepared("go-preflight"))
        .await
        .expect("an unauthenticated Go primary with a usable substitute admits the turn");

    let run = fixture.wait_for_live_run().await;
    assert_eq!(run.model(), SUBSTITUTE_A);
    assert!(run.turn_substitute().is_some());
}

/// An authenticated account whose provider reports every allowance used up until `resets_at_ms`.
fn exhaust_provider_account(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    resets_at_ms: u64,
) -> Result<ProviderAccountProfile, DaemonError> {
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
    };
    crate::test_support::authenticate_provider_account(
        registry,
        owner_user_id,
        provider,
        profile_id,
    )?;
    let now_ms = crate::session::unix_epoch_ms();
    let weekly = ProviderAccountUsageMeter {
        meter_id: "rolling/10080".to_string(),
        label: "Weekly".to_string(),
        service_id: None,
        kind: ProviderAccountUsageMeterKind::RollingLimit,
        scope: ProviderAccountUsageMeterScope::Account,
        used_percent: Some(100.0),
        used: None,
        remaining: None,
        total: None,
        unit: None,
        window_duration_minutes: Some(10080),
        resets_at_ms: Some(resets_at_ms),
        state: ProviderAccountUsageMeterState::Exhausted,
        source: "test.fixture".to_string(),
        observed_at_ms: now_ms,
    };
    // Codex draws credits once its allowance is gone, so it is blocked only
    // when its credits are used up too.
    let credits = ProviderAccountUsageMeter {
        meter_id: "credits".to_string(),
        label: "Credits".to_string(),
        kind: ProviderAccountUsageMeterKind::CreditBalance,
        used_percent: None,
        window_duration_minutes: None,
        resets_at_ms: None,
        ..weekly.clone()
    };
    let meters = if provider == "codex" {
        vec![weekly, credits]
    } else {
        vec![weekly]
    };
    registry.update_usage(
        owner_user_id,
        provider,
        profile_id,
        ProviderAccountUsageSnapshot {
            profile_id: profile_id.to_string(),
            provider: provider.to_string(),
            availability: ProviderAccountUsageAvailability::Available,
            meters,
            observed_at_ms: Some(now_ms),
            source: "test.fixture".to_string(),
            management_url: None,
        },
    )
}


#[tokio::test]
async fn a_weekly_limit_refusal_marks_the_account_exhausted_until_its_reset() {
    crate::test_support::isolated_env_test!();
    let turn = super::automatic_substitutes::failing_turn_on("claude").await;
    crate::test_support::authenticate_provider_account(
        &turn.runtime.owned.provider_account_profiles,
        OWNER,
        "claude",
        "default",
    )
    .unwrap();

    turn.fail_run(
        &turn.failed_run_id,
        "Claude StopFailure [rate_limit]: You've hit your weekly limit \u{b7} resets Nov 22, 4pm (UTC)",
    )
    .await;

    let refusal = turn
        .runtime
        .owned
        .provider_account_profiles
        .require_authenticated(OWNER, "claude", "default", Some("primary-model"), "test")
        .expect_err("the refused account is used up");
    assert!(matches!(
        refusal,
        crate::error::DaemonError::ProviderProfileRefused {
            reason: crate::error::ProviderProfileRefusal::UsageExhausted,
            ..
        }
    ));
}


#[tokio::test]
async fn a_refusal_wins_over_an_older_healthy_credit_reading() {
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
        ProviderAccountUsageSnapshot,
    };
    crate::test_support::isolated_env_test!();
    let turn = super::automatic_substitutes::failing_turn_on("claude").await;
    let registry = &turn.runtime.owned.provider_account_profiles;
    crate::test_support::authenticate_provider_account(registry, OWNER, "claude", "default")
        .unwrap();
    let now_ms = crate::session::unix_epoch_ms();
    let healthy = |meter_id: &str, kind| ProviderAccountUsageMeter {
        meter_id: meter_id.to_string(),
        label: meter_id.to_string(),
        service_id: None,
        kind,
        scope: ProviderAccountUsageMeterScope::Account,
        used_percent: Some(10.0),
        used: None,
        remaining: None,
        total: None,
        unit: None,
        window_duration_minutes: None,
        resets_at_ms: None,
        state: ProviderAccountUsageMeterState::Healthy,
        source: "test.fixture".to_string(),
        observed_at_ms: now_ms,
    };
    registry
        .update_usage(
            OWNER,
            "claude",
            "default",
            ProviderAccountUsageSnapshot {
                profile_id: "default".to_string(),
                provider: "claude".to_string(),
                availability: ProviderAccountUsageAvailability::Available,
                meters: vec![
                    healthy("credits", ProviderAccountUsageMeterKind::CreditBalance),
                    healthy("spend", ProviderAccountUsageMeterKind::SpendLimit),
                ],
                observed_at_ms: Some(now_ms),
                source: "test.fixture".to_string(),
                management_url: None,
            },
        )
        .unwrap();

    registry
        .mark_usage_refused(OWNER, "claude", "default", Some(now_ms + 3_600_000))
        .unwrap();

    let refusal = registry
        .require_authenticated(OWNER, "claude", "default", Some("primary-model"), "test")
        .expect_err("the provider refused the turn, so the account is used up");
    assert!(matches!(
        refusal,
        crate::error::DaemonError::ProviderProfileRefused {
            reason: crate::error::ProviderProfileRefusal::UsageExhausted,
            ..
        }
    ));
    assert!(
        registry
            .get(OWNER, "claude", "default")
            .unwrap()
            .blocked_until("default", now_ms)
            .is_some(),
        "the reset watcher sees the account blocked"
    );
}
