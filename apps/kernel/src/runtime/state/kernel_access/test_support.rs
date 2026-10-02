use super::*;

mod credential_authority;
mod meta_authority;
mod native_launch_authority;
mod session_authority;

impl KernelRuntimeState {
    pub(crate) fn insert_access_grant_for_test(&self, session_id: &str) -> String {
        let session = self.access_session(session_id).unwrap();
        let holder = process::inspect(std::process::id()).unwrap().0;
        let id = format!("queue-access-{:016x}", rand::random::<u64>());
        let grant = Grant {
            summary: KernelAccessGrant {
                grant_id: id.clone(),
                session_id: session_id.into(),
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

    pub(crate) fn access_id_for_test(&self, session: &str, id: String) -> String {
        match std::env::var("CHARIOX_ACCESS_TEST_GRANT_ORDER").as_deref() {
            Ok("ancestor-first") => format!(
                "{}-{id}",
                if session == "access-session" {
                    "a"
                } else {
                    "z"
                }
            ),
            Ok("descendant-first") => format!(
                "{}-{id}",
                if session == "access-session" {
                    "z"
                } else {
                    "a"
                }
            ),
            _ => id,
        }
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
