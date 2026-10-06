use super::*;

mod capability_room_authority;
mod control_authority;
mod credential_authority;
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
            summary: KernelAccessGrant {
                grant_id: id.clone(),
                owner_user_id: session.owner_user_id().into(),
                holder_pid: holder.pid,
                holder_executable: holder.executable.clone(),
                lifetime_minutes: 30,
                expires_at_ms: crate::session::unix_epoch_ms() + 1_800_000,
            },
            holder,
            deadline: Instant::now() + Duration::from_secs(1800),
            notice: Instant::now() + Duration::from_secs(1500),
            notice_sent: false,
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
                "expire" => grant.deadline = Instant::now() - Duration::from_millis(1),
                "notice" => grant.notice = Instant::now() - Duration::from_millis(1),
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
