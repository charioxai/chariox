use super::*;
use crate::local::LocalDaemonResponse;
use crate::skill::v2::{SkillsV2Request, SkillsV2Response};

fn executable(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A Project no provider ever ran in lists the skills of an installed provider: the
/// Catalog with no focused agent carries them, and Details reads one with no agent.
#[test]
fn a_project_lists_an_installed_providers_skills_with_no_run() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("installed-skills");
    let home = worktree.path().join("home");
    std::fs::create_dir_all(home.join(".claude/skills/release-notes")).unwrap();
    std::fs::write(
        home.join(".claude/skills/release-notes/SKILL.md"),
        "---\nname: release-notes\ndescription: Write release notes\n---\nList the changes.\n",
    )
    .unwrap();
    let claude = worktree.path().join("fake-claude");
    executable(&claude, "#!/bin/sh\necho '2.1.0 (Claude Code)'\n");
    std::env::set_var("HOME", &home);
    std::env::set_var("CHARIOX_CLAUDE_BIN", &claude);
    // Codex and OpenCode are not installed on this machine.
    std::env::set_var("CHARIOX_CODEX_BIN", worktree.path().join("no-codex"));
    std::env::set_var("CHARIOX_OPENCODE_BIN", worktree.path().join("no-opencode"));

    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, _) = app.create_session(worktree.session_request()).unwrap();
    let project_id = session.project_id().to_string();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let state = router.runtime_state();
    let user = crate::session::DEFAULT_LOCAL_USER_ID.to_string();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let catalog = runtime
        .block_on(state.execute_skills_catalog_request(
            SkillsV2Request::Catalog {
                project_id: Some(project_id.clone()),
                agent_id: None,
            },
            user.clone(),
        ))
        .unwrap();
    let LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Catalog { native, .. },
    } = catalog
    else {
        panic!("the catalog request answers a catalog");
    };
    let names = native
        .iter()
        .map(|entry| match &entry.selection {
            SkillSelection::Native {
                name,
                provider,
                execution_id,
                ..
            } => (name.clone(), provider.clone(), execution_id.clone()),
            other => panic!("a provider entry is a native selection: {other:?}"),
        })
        .collect::<Vec<_>>();
    let daemon_id = state.owned.config_projection.snapshot().daemon_id;
    assert_eq!(
        names,
        [(
            "release-notes".to_string(),
            "claude".to_string(),
            installed_execution_id("claude", &daemon_id, &project_id),
        )]
    );

    let details = runtime
        .block_on(state.execute_skills_catalog_request(
            SkillsV2Request::NativeDetails {
                agent_id: None,
                selection: native[0].selection.clone(),
            },
            user,
        ))
        .unwrap();
    let LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::NativeDetails { details },
    } = details
    else {
        panic!("Details answers the provider skill");
    };
    assert!(details.body.unwrap().contains("List the changes."));
}

#[test]
fn an_installed_execution_id_names_its_provider_kernel_and_project() {
    let id = installed_execution_id("opencode", "kernel-1", "project:with:colons");
    let parsed = parse_installed_execution_id(&id, "kernel-1")
        .unwrap()
        .unwrap();
    assert_eq!(
        (parsed.provider, parsed.project_id.as_str()),
        ("opencode", "project:with:colons")
    );
    assert!(parse_installed_execution_id(&id, "kernel-2")
        .unwrap()
        .is_err());
    assert!(
        parse_installed_execution_id("installed:gemini:kernel-1:p", "kernel-1")
            .unwrap()
            .is_err()
    );
    assert!(parse_installed_execution_id("kernel-1:run-1", "kernel-1").is_none());
}
