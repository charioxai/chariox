//! A shell or terminal of a Project gets its Variables and present Secrets; a locked Vault leaves only the Secrets out.
use super::*;
use crate::project_environment::*;

#[tokio::test]
async fn a_locked_vault_leaves_secrets_out_of_a_shell_and_a_terminal_but_keeps_the_variables() {
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let workspace = crate::test_support::TestWorktree::new("shell-project-environment");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = config
        .private_runtime_state_root()
        .join("locked-vault.json")
        .to_string_lossy()
        .into_owned();
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    ProjectEnvironmentStore::new(&app.config().private_runtime_state_root())
        .save(
            session.project_id(),
            0,
            ProjectEnvironmentRecord {
                schema: PROJECT_ENVIRONMENT_SCHEMA,
                variables: "LOG_LEVEL=debug".into(),
                secrets: vec!["NOTION_TOKEN".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let shell = runtime.project_environment_for_shell(session.id()).unwrap();
    assert_eq!(
        shell
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>(),
        vec!["LOG_LEVEL".to_string()]
    );
    let terminal = runtime
        .project_environment_for_terminal(session.id())
        .unwrap();
    assert_eq!(terminal.get("LOG_LEVEL").map(String::as_str), Some("debug"));
    assert!(!terminal.contains_key("NOTION_TOKEN"));
}
