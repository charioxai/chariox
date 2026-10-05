//! Per-delivery owner approval, reusing the existing kernel interaction contract.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};

impl KernelRuntimeState {
    pub(super) async fn approve_kernel_browser_fill(
        &self,
        session: &str,
        agent: &str,
        credential: &str,
        target: &SecretTarget,
        admission: &KernelBrowserAdmission,
    ) -> Result<(), DaemonError> {
        let host = &self.owned.kernel_browser_host;
        host.check_admission(Some(admission)).map_err(host_error)?;
        let origin = url::Url::parse(&target.url)
            .map_err(|_| host_error("MD-Vault: invalid target URL".into()))?
            .origin()
            .ascii_serialization();
        // Profile/account authority can normalize a Cloud owner to "local", but
        // the terminal decision must retain the session's actual owner identity.
        let owner = self
            .owned
            .agent_store
            .get_agent(agent)?
            .owner_user_id()
            .to_string();
        let id = format!("kernel-browser-vault-{}", rand::random::<u128>());
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id, format!("vault-fill:{agent}:{id}"),
            "Kernel browser credential fill",
            format!("Allow Vault credential `{credential}` into the observed password field on `{origin}`? This approval permits one document-bound insertion. Navigation or a focus change cancels it."),
            vec![RuntimeInteractionChoice::new("allow", "Fill credential", "allow", Some(RuntimeInteractionChoiceStyle::Primary)).requiring_passkey(),
                 RuntimeInteractionChoice::new("deny", "Cancel", "deny", Some(RuntimeInteractionChoiceStyle::Danger))],
        ).with_timeout_sec(30);
        let id = interaction.id().to_string();
        let mut resolution = self
            .create_kernel_operation_interaction(session, &owner, interaction)
            .await?;
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(30));
        tokio::pin!(deadline);
        let result = loop {
            tokio::select! {
                result = &mut resolution => {
                    break match result {
                        Ok(answer) if answer.status == "answered" && answer.choice_id.as_deref() == Some("allow") => Ok(()),
                        _ => Err(host_error("MD-Vault: fill denied or approval unavailable".into())),
                    };
                }
                _ = &mut deadline => break Err(host_error("MD-Vault: fill approval timed out".into())),
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                    if let Err(message) = host.check_admission(Some(admission)) { break Err(host_error(message)); }
                }
            }
        };
        // Close a denied/cancelled wait without leaving an actionable stale popup.
        if result.is_err() {
            let _ = self.timeout_runtime_interaction(session, &id).await;
        }
        result?;
        host.check_admission(Some(admission)).map_err(host_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;

    #[tokio::test]
    async fn fill_waits_for_explicit_owner_approval_and_focus_revocation_closes_it() {
        for (owner, revoke) in [
            ("local", false),
            ("local", true),
            ("cloud-owner-fixture", false),
        ] {
            let room = TestRoom::new("vault-browser-approval");
            if owner != "local" {
                let mut agent = room
                    .runtime
                    .owned
                    .agent_store
                    .get_agent(&room.agent_id)
                    .unwrap();
                agent.set_owner_user_id(owner);
                room.runtime.owned.agent_store.restore_agent(agent);
                let mut sessions = room.runtime.owned.session_store.write();
                let mut session = sessions.get_session(&room.session_id).unwrap().clone();
                session.set_owner_user_id(owner);
                sessions.restore_session(session);
            }
            let host = &room.runtime.owned.kernel_browser_host;
            host.set_focus("local", Some(&room.agent_id));
            host.load("local", &room.agent_id).unwrap();
            let admission = host.admit("local", &room.agent_id).unwrap();
            let target = SecretTarget {
                url: "https://example.test/login?private-path=never-in-popup".into(),
                target_id: "target".into(),
            };
            let pending = room.runtime.approve_kernel_browser_fill(
                &room.session_id,
                &room.agent_id,
                "fixture-login",
                &target,
                &admission,
            );
            tokio::pin!(pending);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
                    .await
                    .is_err(),
                "unlocked Vault must still require per-fill approval"
            );
            let snapshot = room
                .runtime
                .session_snapshot(&room.session_id)
                .await
                .unwrap();
            let interaction = snapshot.active_interactions().first().unwrap();
            assert_eq!(interaction.title(), Some("Kernel browser credential fill"));
            assert!(!serde_json::to_string(interaction)
                .unwrap()
                .contains("private-path"));
            assert!(
                room.runtime
                    .owned
                    .resolve_runtime_interaction(
                        &room.session_id,
                        interaction.id(),
                        "allow",
                        None,
                        Some("collaborator"),
                        true
                    )
                    .is_err(),
                "only the owner can approve"
            );
            assert!(
                room.runtime
                    .owned
                    .resolve_runtime_interaction(
                        &room.session_id,
                        interaction.id(),
                        "allow",
                        None,
                        Some(owner),
                        false
                    )
                    .is_err(),
                "owner passkey must be verified"
            );
            if revoke {
                host.set_focus("local", None);
                assert!(
                    tokio::time::timeout(std::time::Duration::from_secs(1), &mut pending)
                        .await
                        .unwrap()
                        .is_err()
                );
            } else {
                room.runtime
                    .owned
                    .resolve_runtime_interaction(
                        &room.session_id,
                        interaction.id(),
                        "allow",
                        None,
                        Some(owner),
                        true,
                    )
                    .unwrap();
                pending.await.unwrap();
            }
            assert!(room
                .runtime
                .session_snapshot(&room.session_id)
                .await
                .unwrap()
                .active_interactions()
                .is_empty());
            room.runtime.shutdown_cleanup().await.unwrap();
        }
    }
}
