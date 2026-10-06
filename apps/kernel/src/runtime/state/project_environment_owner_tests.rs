//! MP-08/MP-11: owner copy review is native, compulsory and cancellable.
use super::*;
use crate::config::DaemonConfig;
struct ReviewCleanup(std::path::PathBuf);
impl Drop for ReviewCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mp08_mp11_owner_context_review_rejection_prevents_preparation() {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("owner-context-review".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(owner_context_review_rejection_inner());
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn owner_context_review_rejection_inner() {
    let _guard = crate::env_lock::lock();
    let worktree = crate::test_support::TestWorktree::new("owner-context-review");
    let root = std::env::temp_dir().join(format!(
        "chariox-owner-review-state-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = ReviewCleanup(root.clone());
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config_path = root.join("config.toml");
    config.user_config.history.operational.path =
        Some(root.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config = config.with_session_history_root(root.join("sessions"));
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        8,
    );
    let runtime = router.runtime_state();
    let project = session.project_id().to_string();
    let repository = crate::managed_context::development::DevelopmentRepositorySelection {
        workspace_id: worktree.path().display().to_string(),
        worktree_id: None,
        worktree_path: worktree.path().to_path_buf(),
        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
    };
    let task_runtime = runtime.clone();
    let task = tokio::spawn(async move {
        task_runtime
            .review_credential_free_project_context(&project, &[repository], "Owner machine")
            .await
    });
    let interaction = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if let Some(interaction) = current
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id().starts_with("project-environment:"))
            {
                break interaction.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(interaction.title().unwrap().starts_with("Ready to move "));
    assert!(interaction.title().unwrap().ends_with(" to Owner machine"));
    assert!(interaction
        .choices()
        .iter()
        .any(|choice| choice.label() == "Looks good, continue"));
    runtime
        .resolve_terminal_runtime_interaction(
            session.id(),
            interaction.id(),
            "cancel",
            None,
            Some(session.owner_user_id()),
        )
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}
