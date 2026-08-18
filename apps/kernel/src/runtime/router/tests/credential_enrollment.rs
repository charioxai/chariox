use super::*;

use crate::local::{
    deployment_credential_enrollment_interaction_id,
    deployment_credential_enrollment_service_subject, ArmDeploymentCredentialEnrollmentRequest,
    CredentialEnrollmentInteractionStatus, GetDaemonHealthRequest,
    RequestCredentialEnrollmentInteractionRequest, RespondToInteractionRequest,
};
use crate::session::RuntimeInteractionInputKind;

const REALM_ID: &str = "realm-credential-enrollment";
const PROFILE_ID: &str = "claude-deployment-profile";
const TARGET_VERSION: u64 = 7;
const AUTHORIZATION_URL: &str = "https://claude.com/oauth/authorize?state=opaque-provider-state";

struct EnrollmentTestEnv {
    app: Arc<Mutex<DaemonApp>>,
    local_router: Arc<CommandRouter>,
    relay_router: Arc<CommandRouter>,
    session_id: String,
    agent_id: String,
    attachment_ids: [String; 2],
}

impl EnrollmentTestEnv {
    fn new() -> Self {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "credential-enrollment-workspace",
                "credential-enrollment-worktree",
            ))
            .expect("session should be created");
        let first_attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "credential-client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("first client should attach");
        let second_attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "credential-client-b",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("second client should attach");
        focus_test_agent(&mut app, session.id(), agent.id());

        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_ids = [
            first_attachment.id().to_string(),
            second_attachment.id().to_string(),
        ];
        let app = Arc::new(Mutex::new(app));
        let local_router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::clone(&app),
            4,
        ));
        let relay_router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::clone(&app),
            4,
        ));
        Self {
            app,
            local_router,
            relay_router,
            session_id,
            agent_id,
            attachment_ids,
        }
    }

    fn arm_request(&self, enrollment_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::ArmDeploymentCredentialEnrollment(
            ArmDeploymentCredentialEnrollmentRequest {
                session_id: self.session_id.clone(),
                attachment_id: self.attachment_ids[0].clone(),
                agent_id: self.agent_id.clone(),
                enrollment_id: enrollment_id.to_string(),
                profile_id: PROFILE_ID.to_string(),
                target_version: TARGET_VERSION,
            },
        )
    }

    async fn arm(&self, enrollment_id: &str) {
        let request = self.arm_request(enrollment_id);
        let command = client_command(
            &format!("arm-{enrollment_id}"),
            "credential-client-a",
            &request,
        );
        let router = Arc::clone(&self.local_router);
        let armed = tokio::spawn(async move {
            dispatch_boxed(&router, command, request)
                .await
                .map(|response| {
                    matches!(
                        response,
                        LocalDaemonResponse::DeploymentCredentialEnrollmentArmed { .. }
                    )
                })
        })
        .await
        .expect("credential enrollment arm task should join")
        .expect("credential enrollment should arm");
        assert!(armed, "credential enrollment should return an arm response");
    }

    fn interaction_request(&self, enrollment_id: &str, timeout_sec: u64) -> LocalDaemonRequest {
        LocalDaemonRequest::RequestCredentialEnrollmentInteraction(
            RequestCredentialEnrollmentInteractionRequest {
                session_id: self.session_id.clone(),
                agent_id: self.agent_id.clone(),
                enrollment_id: enrollment_id.to_string(),
                profile_id: PROFILE_ID.to_string(),
                target_version: TARGET_VERSION,
                provider_authorization_url: AUTHORIZATION_URL.to_string(),
                timeout_sec: Some(timeout_sec),
            },
        )
    }

    async fn wait_for_interaction(&self) -> RuntimeInteraction {
        timeout(Duration::from_secs(1), async {
            loop {
                let session = self
                    .relay_router
                    .runtime_state
                    .session_snapshot_projection(&self.session_id, 0)
                    .expect("session projection should resolve")
                    .session;
                if let Some(interaction) = session.active_interactions().first() {
                    return interaction.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("credential interaction should become active")
    }

    async fn subscription_interaction(&self, attachment_id: &str) -> RuntimeInteraction {
        match self
            .relay_router
            .relay_watch_subscription_state(&self.session_id, attachment_id, true, None, 0)
            .await
        {
            crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
                .as_ref()
                .as_ref()
                .expect("subscription should include a snapshot")
                .session
                .active_interactions()
                .first()
                .cloned()
                .expect("subscription should project the credential interaction"),
            crate::runtime_transport::WatchResult::Unavailable(message) => {
                panic!("subscription unavailable: {message}")
            }
        }
    }
}

fn armed_expiry(response: LocalDaemonResponse) -> u64 {
    match response {
        LocalDaemonResponse::DeploymentCredentialEnrollmentArmed { expires_at_ms, .. } => {
            expires_at_ms
        }
        response => panic!("expected credential enrollment arm response, got {response:?}"),
    }
}

fn client_command(
    command_id: &str,
    caller_id: &str,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    KernelCommand::from_local_request_with_caller(
        command_id,
        KernelCommandSource::LocalCli,
        KernelCaller {
            caller_id: caller_id.to_string(),
            caller_kind: KernelCallerKind::LocalClient,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: Some(caller_id.to_string()),
            machine_id: None,
            realm_id: Some(REALM_ID.to_string()),
            public_key_thumbprint: None,
            metaagent_id: None,
        },
        None,
        None,
        request,
    )
}

async fn dispatch_boxed(
    router: &CommandRouter,
    command: KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Box::pin(router.dispatch(command, request)).await
}

fn service_command(
    command_id: &str,
    enrollment_id: &str,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    service_command_with_subject(
        command_id,
        &deployment_credential_enrollment_service_subject(enrollment_id),
        request,
    )
}

fn service_command_with_subject(
    command_id: &str,
    subject: &str,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    KernelCommand::from_local_request_with_caller(
        command_id,
        KernelCommandSource::RelayClient,
        KernelCaller {
            caller_id: subject.to_string(),
            caller_kind: KernelCallerKind::HostedService,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: None,
            machine_id: None,
            realm_id: Some(REALM_ID.to_string()),
            public_key_thumbprint: Some("verified-service-key".to_string()),
            metaagent_id: None,
        },
        None,
        None,
        request,
    )
}

fn interaction_response(
    session_id: &str,
    interaction_id: &str,
    callback: &str,
) -> LocalDaemonRequest {
    LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: session_id.to_string(),
        interaction_id: interaction_id.to_string(),
        choice_id: "submit_callback".to_string(),
        custom_reply: Some(callback.to_string()),
    })
}

async fn resolve_cancel(env: &EnrollmentTestEnv, interaction_id: &str, command_id: &str) {
    let request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: env.session_id.clone(),
        interaction_id: interaction_id.to_string(),
        choice_id: "cancel".to_string(),
        custom_reply: None,
    });
    dispatch_boxed(
        &env.local_router,
        client_command(command_id, "credential-client-a", &request),
        request,
    )
    .await
    .expect("cancel should resolve the interaction");
}

#[tokio::test]
async fn credential_enrollment_projects_to_two_clients_and_first_reply_wins() {
    Box::pin(run_two_client_credential_enrollment_scenario()).await;
}

async fn run_two_client_credential_enrollment_scenario() {
    const CALLBACK_A: &str = "https://localhost/callback?code=secret-callback-a";
    const CALLBACK_B: &str = "https://localhost/callback?code=secret-callback-b";

    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-two-clients";
    env.arm(enrollment_id).await;

    let helper_request = env.interaction_request(enrollment_id, 30);
    let helper_command = service_command("helper-two-clients", enrollment_id, &helper_request);
    let helper_router = env.relay_router.clone();
    let helper_task = tokio::spawn(async move {
        let response = dispatch_boxed(&helper_router, helper_command, helper_request)
            .await
            .expect("helper request should resolve");
        assert!(!format!("{response:?}").contains("secret-callback"));
        match response {
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved { status, callback } => (
                status,
                callback.map(|callback| callback.expose_secret().to_string()),
            ),
            response => panic!("unexpected helper response: {response:?}"),
        }
    });

    let interaction = env.wait_for_interaction().await;
    assert!(interaction.message().contains(AUTHORIZATION_URL));
    let custom_choice = interaction
        .custom_choice()
        .expect("credential interaction should accept a callback");
    assert_eq!(custom_choice.id(), "submit_callback");
    assert_eq!(
        custom_choice.input_kind(),
        RuntimeInteractionInputKind::Secret
    );

    let first_projection = env.subscription_interaction(&env.attachment_ids[0]).await;
    let second_projection = env.subscription_interaction(&env.attachment_ids[1]).await;
    assert_eq!(first_projection, interaction);
    assert_eq!(second_projection, interaction);

    let response_a = interaction_response(&env.session_id, interaction.id(), CALLBACK_A);
    let response_b = interaction_response(&env.session_id, interaction.id(), CALLBACK_B);
    let router_a = env.local_router.clone();
    let router_b = env.relay_router.clone();
    let command_a = client_command("reply-client-a", "credential-client-a", &response_a);
    let command_b = client_command("reply-client-b", "credential-client-b", &response_b);
    let first_reply = tokio::spawn(async move {
        dispatch_boxed(&router_a, command_a, response_a)
            .await
            .map(|_| ())
    })
    .await
    .expect("first reply task should join");
    assert!(first_reply.is_ok(), "the first attached client should win");
    let second_reply = tokio::spawn(async move {
        dispatch_boxed(&router_b, command_b, response_b)
            .await
            .map(|_| ())
    })
    .await
    .expect("second reply task should join");
    assert!(
        second_reply.is_err(),
        "the second attached client must observe the resolved interaction"
    );

    let (helper_status, helper_callback) = helper_task.await.expect("helper task should join");
    assert_eq!(
        helper_status,
        CredentialEnrollmentInteractionStatus::Submitted
    );
    assert_eq!(
        helper_callback
            .as_deref()
            .expect("submitted callback should be returned"),
        CALLBACK_A
    );

    let session = env
        .relay_router
        .runtime_state
        .session_snapshot_projection(&env.session_id, 0)
        .expect("session projection should resolve")
        .session;
    assert!(session.active_interactions().is_empty());
    let serialized_session = serde_json::to_string(&session).expect("session should serialize");
    assert!(!serialized_session.contains(CALLBACK_A));
    assert!(!serialized_session.contains(CALLBACK_B));
    let serialized_events =
        serde_json::to_string(&env.app.lock().await.metaagent_event_store().snapshot())
            .expect("events should serialize");
    assert!(!serialized_events.contains(CALLBACK_A));
    assert!(!serialized_events.contains(CALLBACK_B));

    let replay_request = env.interaction_request(enrollment_id, 30);
    let replay_command =
        service_command("helper-two-clients-replay", enrollment_id, &replay_request);
    let replay_router = Arc::clone(&env.relay_router);
    let replay_rejected = tokio::spawn(async move {
        dispatch_boxed(&replay_router, replay_command, replay_request)
            .await
            .is_err()
    })
    .await
    .expect("replay task should join");
    assert!(
        replay_rejected,
        "consumed enrollment arms must reject replay"
    );
}

#[tokio::test]
async fn credential_enrollment_cancel_and_timeout_return_no_callback() {
    Box::pin(run_credential_enrollment_cancel_and_timeout_scenario()).await;
}

async fn run_credential_enrollment_cancel_and_timeout_scenario() {
    let env = EnrollmentTestEnv::new();

    let cancel_enrollment_id = "enrollment-cancel";
    env.arm(cancel_enrollment_id).await;
    let cancel_request = env.interaction_request(cancel_enrollment_id, 30);
    let cancel_command = service_command(
        "helper-enrollment-cancel",
        cancel_enrollment_id,
        &cancel_request,
    );
    let cancel_router = env.relay_router.clone();
    let cancel_task = tokio::spawn(async move {
        dispatch_boxed(&cancel_router, cancel_command, cancel_request).await
    });
    let cancel_interaction = env.wait_for_interaction().await;
    resolve_cancel(&env, cancel_interaction.id(), "cancel-enrollment").await;
    assert!(matches!(
        cancel_task.await.expect("cancel task should join"),
        Ok(
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                status: CredentialEnrollmentInteractionStatus::Canceled,
                callback: None,
            }
        )
    ));

    let timeout_enrollment_id = "enrollment-timeout";
    env.arm(timeout_enrollment_id).await;
    let timeout_request = env.interaction_request(timeout_enrollment_id, 1);
    let timeout_response = dispatch_boxed(
        &env.relay_router,
        service_command(
            "helper-enrollment-timeout",
            timeout_enrollment_id,
            &timeout_request,
        ),
        timeout_request,
    )
    .await
    .expect("timed interaction should return a resolution");
    assert!(matches!(
        timeout_response,
        LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
            status: CredentialEnrollmentInteractionStatus::TimedOut,
            callback: None,
        }
    ));
    assert!(env
        .relay_router
        .runtime_state
        .session_snapshot_projection(&env.session_id, 0)
        .expect("session projection should resolve")
        .session
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn matching_credential_service_can_cancel_only_its_own_interaction() {
    Box::pin(run_matching_credential_service_cancel_scenario()).await;
}

async fn run_matching_credential_service_cancel_scenario() {
    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-worker-cancel";
    env.arm(enrollment_id).await;

    let helper_request = env.interaction_request(enrollment_id, 30);
    let helper_command = service_command("helper-worker-cancel", enrollment_id, &helper_request);
    let helper_router = Arc::clone(&env.relay_router);
    let helper_task = tokio::spawn(async move {
        dispatch_boxed(&helper_router, helper_command, helper_request).await
    });
    let interaction = env.wait_for_interaction().await;
    assert_eq!(
        interaction.id(),
        deployment_credential_enrollment_interaction_id(enrollment_id)
    );

    for (command_id, subject, interaction_id, choice_id, custom_reply) in [
        (
            "worker-cancel-wrong-subject",
            deployment_credential_enrollment_service_subject("other-enrollment"),
            interaction.id().to_string(),
            "cancel",
            None,
        ),
        (
            "worker-cancel-wrong-interaction",
            deployment_credential_enrollment_service_subject(enrollment_id),
            deployment_credential_enrollment_interaction_id("other-enrollment"),
            "cancel",
            None,
        ),
        (
            "worker-cancel-wrong-choice",
            deployment_credential_enrollment_service_subject(enrollment_id),
            interaction.id().to_string(),
            "submit_callback",
            None,
        ),
        (
            "worker-cancel-with-reply",
            deployment_credential_enrollment_service_subject(enrollment_id),
            interaction.id().to_string(),
            "cancel",
            Some("must-not-be-accepted".to_string()),
        ),
    ] {
        let request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
            session_id: env.session_id.clone(),
            interaction_id,
            choice_id: choice_id.to_string(),
            custom_reply,
        });
        let error = dispatch_boxed(
            &env.relay_router,
            service_command_with_subject(command_id, &subject, &request),
            request,
        )
        .await
        .expect_err("mismatched service cancellation must be denied");
        assert!(error
            .to_string()
            .contains("hosted service identity is not authorized"));
    }

    let cancel_request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: env.session_id.clone(),
        interaction_id: interaction.id().to_string(),
        choice_id: "cancel".to_string(),
        custom_reply: None,
    });
    let cancel_response = dispatch_boxed(
        &env.relay_router,
        service_command("worker-cancel-exact", enrollment_id, &cancel_request),
        cancel_request,
    )
    .await
    .expect("matching service should cancel its enrollment interaction");
    assert!(matches!(
        cancel_response,
        LocalDaemonResponse::InteractionResponded { .. }
    ));
    assert!(matches!(
        helper_task.await.expect("helper task should join"),
        Ok(
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                status: CredentialEnrollmentInteractionStatus::Canceled,
                callback: None,
            }
        )
    ));
    assert!(env
        .relay_router
        .runtime_state
        .session_snapshot_projection(&env.session_id, 0)
        .expect("session projection should resolve")
        .session
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn credential_enrollment_rejects_unverified_and_wrong_service_subject() {
    Box::pin(run_credential_service_subject_rejection_scenario()).await;
}

async fn run_credential_service_subject_rejection_scenario() {
    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-service-identity";
    env.arm(enrollment_id).await;
    let request = env.interaction_request(enrollment_id, 30);

    let unverified_command = KernelCommand::from_local_request_with_caller(
        "helper-unverified",
        KernelCommandSource::RelayClient,
        KernelCaller {
            caller_id: deployment_credential_enrollment_service_subject(enrollment_id),
            caller_kind: KernelCallerKind::RemoteClient,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: Some("unverified-client".to_string()),
            machine_id: None,
            realm_id: Some(REALM_ID.to_string()),
            public_key_thumbprint: None,
            metaagent_id: None,
        },
        None,
        None,
        &request,
    );
    assert!(
        dispatch_boxed(&env.relay_router, unverified_command, request.clone())
            .await
            .is_err()
    );
    assert!(dispatch_boxed(
        &env.relay_router,
        service_command_with_subject("helper-wrong-subject", "wrong-service", &request),
        request.clone(),
    )
    .await
    .is_err());

    let correct_command = service_command("helper-correct-subject", enrollment_id, &request);
    let correct_router = env.relay_router.clone();
    let correct_task =
        tokio::spawn(
            async move { dispatch_boxed(&correct_router, correct_command, request).await },
        );
    let interaction = env.wait_for_interaction().await;
    resolve_cancel(&env, interaction.id(), "cancel-correct-subject").await;
    assert!(correct_task
        .await
        .expect("correct helper task should join")
        .is_ok());
}

#[tokio::test]
async fn credential_enrollment_service_identity_cannot_invoke_other_kernel_requests() {
    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-service-scope";
    let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let command = service_command("helper-health-probe", enrollment_id, &request);

    let error = dispatch_boxed(&env.relay_router, command, request)
        .await
        .expect_err("hosted service identity must be request-scoped");
    assert!(error
        .to_string()
        .contains("hosted service identity is not authorized"));
}

#[tokio::test]
async fn credential_enrollment_arm_requires_attached_focused_target() {
    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-arm-target";

    let wrong_attachment = LocalDaemonRequest::ArmDeploymentCredentialEnrollment(
        ArmDeploymentCredentialEnrollmentRequest {
            session_id: env.session_id.clone(),
            attachment_id: "attachment-not-in-session".to_string(),
            agent_id: env.agent_id.clone(),
            enrollment_id: enrollment_id.to_string(),
            profile_id: PROFILE_ID.to_string(),
            target_version: TARGET_VERSION,
        },
    );
    assert!(dispatch_boxed(
        &env.local_router,
        client_command(
            "arm-wrong-attachment",
            "credential-client-a",
            &wrong_attachment,
        ),
        wrong_attachment,
    )
    .await
    .is_err());

    let wrong_agent = LocalDaemonRequest::ArmDeploymentCredentialEnrollment(
        ArmDeploymentCredentialEnrollmentRequest {
            session_id: env.session_id.clone(),
            attachment_id: env.attachment_ids[0].clone(),
            agent_id: "agent-not-focused".to_string(),
            enrollment_id: enrollment_id.to_string(),
            profile_id: PROFILE_ID.to_string(),
            target_version: TARGET_VERSION,
        },
    );
    assert!(dispatch_boxed(
        &env.local_router,
        client_command("arm-wrong-agent", "credential-client-a", &wrong_agent),
        wrong_agent,
    )
    .await
    .is_err());

    env.arm(enrollment_id).await;
}

#[tokio::test]
async fn credential_enrollment_rearm_retries_only_the_exact_pending_route() {
    Box::pin(run_credential_enrollment_rearm_scenario()).await;
}

async fn run_credential_enrollment_rearm_scenario() {
    let env = EnrollmentTestEnv::new();
    let enrollment_id = "enrollment-arm-retry";
    let original_request = env.arm_request(enrollment_id);
    let original_expiry = armed_expiry(
        dispatch_boxed(
            &env.local_router,
            client_command(
                "arm-retry-interrupted",
                "credential-client-a",
                &original_request,
            ),
            original_request.clone(),
        )
        .await
        .expect("initial arm should succeed before its response is lost"),
    );

    let retry_expiry = armed_expiry(
        dispatch_boxed(
            &env.local_router,
            client_command(
                "arm-retry-resumed",
                "credential-client-a",
                &original_request,
            ),
            original_request.clone(),
        )
        .await
        .expect("the exact retry should recover the pending arm"),
    );
    assert_eq!(retry_expiry, original_expiry);

    let mut mismatched_request = original_request.clone();
    let LocalDaemonRequest::ArmDeploymentCredentialEnrollment(request) = &mut mismatched_request
    else {
        unreachable!("arm fixture must contain an enrollment request");
    };
    request.profile_id = "different-profile".to_string();
    assert!(dispatch_boxed(
        &env.local_router,
        client_command(
            "arm-retry-route-mismatch",
            "credential-client-a",
            &mismatched_request,
        ),
        mismatched_request,
    )
    .await
    .is_err());

    assert_eq!(
        armed_expiry(
            dispatch_boxed(
                &env.local_router,
                client_command(
                    "arm-retry-original-route",
                    "credential-client-a",
                    &original_request,
                ),
                original_request.clone(),
            )
            .await
            .expect("a mismatch must not replace the original pending route"),
        ),
        original_expiry
    );

    let helper_request = env.interaction_request(enrollment_id, 30);
    let helper_command = service_command("arm-retry-helper", enrollment_id, &helper_request);
    let helper_router = Arc::clone(&env.relay_router);
    let helper_task = tokio::spawn(async move {
        dispatch_boxed(&helper_router, helper_command, helper_request).await
    });
    let interaction = env.wait_for_interaction().await;
    resolve_cancel(&env, interaction.id(), "arm-retry-cancel").await;
    assert!(matches!(
        helper_task.await.expect("helper task should join"),
        Ok(
            LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
                status: CredentialEnrollmentInteractionStatus::Canceled,
                callback: None,
            }
        )
    ));

    assert!(dispatch_boxed(
        &env.local_router,
        client_command(
            "arm-retry-after-cancel",
            "credential-client-a",
            &original_request,
        ),
        original_request,
    )
    .await
    .is_err());
}

#[tokio::test]
async fn busy_interaction_consumes_arm_and_replay_fails_closed() {
    let env = EnrollmentTestEnv::new();
    let busy_interaction = RuntimeInteraction::new(
        "existing-busy-interaction",
        &env.agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Existing interaction".to_string()),
        "Resolve the existing interaction first",
        vec![RuntimeInteractionChoice::new(
            "ok",
            "OK",
            "ok",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let busy_receiver = env
        .local_router
        .runtime_state
        .create_runtime_interaction(&env.session_id, busy_interaction)
        .await
        .expect("busy interaction should register");

    let enrollment_id = "enrollment-busy";
    env.arm(enrollment_id).await;
    let request = env.interaction_request(enrollment_id, 30);
    assert!(
        dispatch_boxed(
            &env.relay_router,
            service_command("helper-busy", enrollment_id, &request),
            request.clone(),
        )
        .await
        .is_err(),
        "busy agents must reject a second interaction"
    );
    assert!(
        dispatch_boxed(
            &env.relay_router,
            service_command("helper-busy-replay", enrollment_id, &request),
            request,
        )
        .await
        .is_err(),
        "a failed busy request must not be replayable"
    );

    env.local_router
        .runtime_state
        .resolve_runtime_interaction(&env.session_id, "existing-busy-interaction", "ok", None)
        .await
        .expect("busy interaction should resolve");
    busy_receiver
        .await
        .expect("busy interaction resolution should be delivered");
}
