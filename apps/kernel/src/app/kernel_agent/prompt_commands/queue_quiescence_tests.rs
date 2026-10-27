use std::sync::Arc;

use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::DaemonConfig;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::provider::LaunchProviderRequest;
use crate::runtime::router::CommandRouter;
use crate::runtime::state::{
    KernelRuntimeState, ManagedKernelQuiescenceChallenge, ManagedKernelQuiescenceOutcome,
};
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::DaemonApp;
use tokio::sync::Mutex;

struct Fixture {
    _worktree: crate::test_support::TestWorktree,
    app: Arc<Mutex<DaemonApp>>,
    _router: CommandRouter,
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    provider_run_id: String,
    challenge: ManagedKernelQuiescenceChallenge,
}

fn fixture(label: &str) -> Fixture {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let worktree = crate::test_support::TestWorktree::new(label);
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            format!("client-{label}"),
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should be created");
    let provider_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_agent_id(agent.id()),
        )
        .expect("idle provider should launch");

    let kernel_id = app.config().daemon_id.clone();
    app.managed_kernel_registration = Some(ConfirmedManagedKernelRegistration {
        environment_id: "queue-test-environment".to_string(),
        machine_id: "queue-test-machine".to_string(),
        kernel_id: kernel_id.clone(),
        context_plan: None,
    });
    let provider_lanes = app.provider_run_operation_lanes();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        16,
        provider_lanes,
    );
    let runtime = router.runtime_state();
    runtime
        .ensure_managed_activity_tracking(&kernel_id)
        .expect("managed activity tracking should initialize");
    let (_, observation, local_transition_sequence) = runtime
        .managed_activity_report_snapshot_with_transition()
        .expect("idle activity should be readable");
    assert_eq!(observation.running_agent_count, 0);

    let challenge = ManagedKernelQuiescenceChallenge {
        challenge_id: format!("challenge-{label}"),
        account_id: "queue-test-account".to_string(),
        environment_id: "queue-test-environment".to_string(),
        machine_id: "queue-test-machine".to_string(),
        kernel_id,
        desired_revision: 1,
        idle_sequence: 7,
        idle_deadline_at: "2026-09-26T00:00:00.000Z".to_string(),
        stop_operation_id: format!("stop-{label}"),
        nonce: format!("nonce-{label}"),
    };
    runtime.confirm_managed_activity_report(
        challenge.idle_sequence,
        local_transition_sequence,
        observation,
    );
    assert!(runtime
        .reserve_managed_kernel_for_stop(challenge.clone())
        .expect("idle admission fence should persist"));

    Fixture {
        _worktree: worktree,
        app,
        _router: router,
        runtime,
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        attachment_id: attachment.id().to_string(),
        provider_run_id: provider_run.id().to_string(),
        challenge,
    }
}

fn seed_active(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    agent_id: &str,
    id: &str,
) {
    // This fixture represents already-owned work under the fence. Ordinary
    // admission must keep FIFO instead of starting a new turn over its backlog.
    app.prompt_owner_activate_prompt(
        session_id,
        PromptQueueItem::new(id, attachment_id, agent_id, id, PromptStatus::Running),
    )
    .expect("already-owned active prompt should be seeded");
}

#[test]
fn app_queue_promotion_stays_pending_until_exact_release_then_dispatches_once() {
    let fixture = fixture("quiescence-app-queue-release");
    let mut app = fixture.app.blocking_lock();

    // Seed already-owned app work behind the fence so this test isolates promotion.
    seed_active(
        &mut app,
        &fixture.session_id,
        &fixture.attachment_id,
        &fixture.agent_id,
        "complete-before-release",
    );
    let queued = match app
        .prompt_owner_submit_prepared_prompt(
            &fixture.session_id,
            PromptQueueItem::new(
                "queued-after-fence",
                &fixture.attachment_id,
                &fixture.agent_id,
                "deliver exactly once after the release",
                PromptStatus::Queued,
            ),
            true,
        )
        .expect("queued prompt should remain accepted")
    {
        PromptSubmissionOutcome::Queued { prompt } => prompt,
        PromptSubmissionOutcome::Started { .. } => panic!("forced prompt should remain queued"),
    };

    let completed = app
        .prompt_owner_complete_active_prompt_only(&fixture.session_id, &fixture.agent_id)
        .expect("completion should remain available under the fence");
    assert_eq!(completed.status(), PromptStatus::Completed);
    assert!(crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
        .expect("completion must not be blocked by the admission fence")
        .is_none());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(&fixture.session_id, &fixture.agent_id)
            .expect("queued prompt count should resolve"),
        1,
        "completion must leave promotion pending while fenced"
    );

    seed_active(
        &mut app,
        &fixture.session_id,
        &fixture.attachment_id,
        &fixture.agent_id,
        "cancel-before-release",
    );
    app.prompt_owner_begin_cancelling_active_prompt(&fixture.session_id, &fixture.agent_id)
        .expect("cancellation request should remain available under the fence");
    let cancelled = app
        .prompt_owner_cancel_active_prompt_only(&fixture.session_id, &fixture.agent_id)
        .expect("cancellation settlement should remain available under the fence");
    assert_eq!(cancelled.status(), PromptStatus::Cancelled);
    assert!(crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
        .expect("cancellation must not be blocked by the admission fence")
        .is_none());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(&fixture.session_id, &fixture.agent_id)
            .expect("queued prompt count should resolve"),
        1,
        "cancellation settlement must leave promotion pending while fenced"
    );

    let input_count_before_release = app.terminal().input_records().len();
    assert_eq!(
        crate::app::KernelAgentService::new(&mut app)
            .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
            .expect("fenced app queue promotion should be deferred"),
        None,
        "the app route must not activate queued work while admission is fenced"
    );
    assert_eq!(
        app.prompt_owner_active_prompt_for_agent(&fixture.session_id, &fixture.agent_id)
            .expect("active prompt should resolve"),
        None,
        "blocked promotion must not create an active mirror prompt"
    );
    assert_eq!(
        app.terminal().input_records().len(),
        input_count_before_release,
        "blocked promotion must not write prompt bytes to the provider"
    );

    let mut wrong_challenge = fixture.challenge.clone();
    wrong_challenge.nonce.push_str("-wrong");
    assert!(fixture
        .runtime
        .apply_managed_kernel_stop_release(
            &wrong_challenge,
            ManagedKernelQuiescenceOutcome::Stopped,
            1,
        )
        .is_err());
    assert!(crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
        .expect("mismatched release must leave the queue blocked")
        .is_none());

    fixture
        .runtime
        .apply_managed_kernel_stop_release(
            &fixture.challenge,
            ManagedKernelQuiescenceOutcome::Stopped,
            1,
        )
        .expect("exact terminal STOP receipt should persist without opening admission");
    assert!(crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
        .expect("terminal STOP receipt must retain the queue fence")
        .is_none());
    fixture
        .runtime
        .apply_managed_kernel_stop_release(
            &fixture.challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            2,
        )
        .expect("exact newer keep-running release should open admission");

    let promoted = crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, Some(&queued))
        .expect("matching release should resume the app queue")
        .expect("one queued prompt should promote");
    assert_eq!(promoted.prompt(), queued.prompt());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(&fixture.session_id, &fixture.agent_id)
            .expect("queued prompt count should resolve"),
        0
    );
    let inputs_after_promotion = app.terminal().input_records();
    assert_eq!(
        inputs_after_promotion.len(),
        input_count_before_release + 1,
        "one released promotion should dispatch exactly once"
    );
    assert!(inputs_after_promotion.last().is_some_and(|record| {
        record.provider_run_id == fixture.provider_run_id
            && String::from_utf8_lossy(&record.bytes).contains(queued.prompt())
    }));

    assert!(crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(&fixture.session_id, &fixture.agent_id, None)
        .expect("completed queue should not dispatch again")
        .is_none());
    assert_eq!(
        app.terminal().input_records().len(),
        inputs_after_promotion.len(),
        "repeated advancement must not write another provider prompt"
    );
}
