use super::*;

mod config_invalidation;
mod destroy_cleanup;
mod spawn_owned_runtime;
mod spawn_validation_and_placement;

fn temp_git_repo(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "arroba-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.invalid"]);
    run_git(&root, &["config", "user.name", "Arroba Tests"]);
    std::fs::write(root.join("README.md"), "worktree placement\n")
        .expect("fixture file should be written");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "initial"]);
    root
}

fn external_provider_session_record(
    provider: &str,
    provider_session_id: &str,
    last_modified_at_ms: u64,
) -> ExternalProviderSessionRecord {
    ExternalProviderSessionRecord {
        external_session_id: format!("{provider}:{provider_session_id}"),
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        title: Some(provider_session_id.to_string()),
        title_source: Some("test".to_string()),
        first_prompt_preview: None,
        created_at_ms: None,
        last_modified_at_ms,
        worktree_path: None,
        account_profile: None,
        capabilities: ExternalProviderSessionCapabilities {
            ..ExternalProviderSessionCapabilities::default()
        },
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}

fn git_output(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
