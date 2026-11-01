//! MP-08 / MP-09 / MP-10 / MP-11: an owner's plain approval keeps the requested term.
use super::*;

#[tokio::test]
async fn kernel_access_permission_approval_keeps_the_requested_lifetime() {
    for (requested, reply, expected) in [
        (
            Some(KernelAccessTerm::UntilHolderExits),
            None,
            KernelAccessTerm::UntilHolderExits,
        ),
        (
            Some(KernelAccessTerm::Minutes(15)),
            None,
            KernelAccessTerm::Minutes(15),
        ),
        (None, None, KernelAccessTerm::Minutes(30)),
        (
            Some(KernelAccessTerm::Minutes(15)),
            Some("until_exit"),
            KernelAccessTerm::UntilHolderExits,
        ),
        (
            Some(KernelAccessTerm::UntilHolderExits),
            Some("10"),
            KernelAccessTerm::Minutes(10),
        ),
    ] {
        let root = crate::test_support::TestWorktree::new("access-plain-approval");
        let vault = root.path().join("vault.json");
        const PASSKEY: &str = "Access TEST Passkey";
        crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault = crate::config::UserCredentialVaultConfig {
            backend: crate::config::CredentialVaultBackend::CharioxEncrypted,
            path: vault.display().to_string(),
            ..Default::default()
        };
        config.user_config.kernel_access.grant_default_minutes = 30;
        let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            32,
        );
        let state = router.runtime_state();
        // The test runner's parent is outside the kernel, and remains live for this request.
        let (_, parent) = process::inspect(std::process::id()).unwrap();
        let holder = process::inspect(parent).unwrap().0;
        let request = state.request_kernel_access(
            holder.clone(),
            RequestKernelAccessRequest {
                holder_pid: holder.pid,
                term: requested,
            },
        );
        let answer = async {
            let prompt = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(prompt) = state
                        .passkey_prompts_for("local")
                        .into_iter()
                        .find(|p| p.kind == PasskeyPromptKind::AccessGrant)
                    {
                        break prompt;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("access popup should appear");
            state
                .answer_terminal_runtime_interaction(
                    &prompt.session_id,
                    &prompt.interaction_id,
                    "approve",
                    reply,
                    Some("local"),
                    Some(&ApprovalPasskey::new(PASSKEY)),
                    None,
                    Some(KernelConnectionClass::Terminal),
                )
                .await
                .expect("the owner's approval should resolve");
        };
        let (grant, ()) = tokio::join!(request, answer);
        let grant = grant.expect("plain approval should create a live grant");
        assert_eq!(grant.term, expected);
        assert_eq!(
            grant.expires_at_ms.is_some(),
            matches!(expected, KernelAccessTerm::Minutes(_))
        );
        assert!(state
            .owned
            .kernel_access
            .lock()
            .unwrap()
            .grants
            .contains_key(&grant.grant_id));
        assert!(state.passkey_prompts_for("local").is_empty());
    }
}
