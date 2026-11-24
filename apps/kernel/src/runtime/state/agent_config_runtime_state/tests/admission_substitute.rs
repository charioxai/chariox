//! A turn whose primary account is refused at admission (usage exhausted or
//! not signed in) runs on the first usable substitute instead of waiting for
//! the primary, and the Room hears about each state change once.

use super::*;
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
    worktree: std::path::PathBuf,
}

impl Drop for RefusedPrimary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.worktree);
    }
}

fn far_future_ms() -> u64 {
    crate::session::unix_epoch_ms() + 6 * 24 * 60 * 60 * 1000
}

/// A Codex agent whose `default` account is exhausted, with `substitutes`
/// (dev-stub models) configured in order.
async fn refused_primary(substitutes: &[&str]) -> RefusedPrimary {
    let worktree = crate::test_support::test_workspace_root().join(format!(
        "chariox-admission-substitute-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&worktree).unwrap();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            worktree.to_string_lossy(),
            worktree.to_string_lossy(),
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
    crate::test_support::exhaust_provider_account(
        &runtime.owned.provider_account_profiles,
        OWNER,
        "codex",
        "default",
        far_future_ms(),
    )
    .unwrap();
    RefusedPrimary {
        runtime,
        session_id,
        agent_id,
        attachment_id,
        worktree,
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

// S3: a provider that refuses a turn for the weekly limit marks the account
// used up, with the reset it names, so admission skips it from then on.
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

// A provider refusal is the newest word on the account: a healthy credit reading
// taken before it does not keep the account admitted.
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
