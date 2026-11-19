use super::*;
use crate::local::LocalDaemonResponse;
use crate::skill::v2::{SkillsV2Request, SkillsV2Response};

fn executable(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A Project whose host owner has `release-notes` in the Claude account, with Claude
/// installed and Codex and OpenCode not, and one Room whose agent never started.
struct Installed {
    _worktree: crate::test_support::TestWorktree,
    state: KernelRuntimeState,
    runtime: tokio::runtime::Runtime,
    session_id: String,
    agent_id: String,
    project_id: String,
}

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

impl Installed {
    fn new(agent_provider: &str) -> Self {
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
        let mut request = worktree.session_request();
        request.agent_defaults = Some(crate::session::SessionAgentDefaults {
            provider: agent_provider.into(),
            ..Default::default()
        });
        let (session, agent) = app.create_session(request).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        Self {
            _worktree: worktree,
            state: router.runtime_state(),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            project_id: session.project_id().to_string(),
        }
    }

    fn request(
        &self,
        request: SkillsV2Request,
        user: &str,
    ) -> Result<SkillsV2Response, DaemonError> {
        match self.runtime.block_on(
            self.state
                .execute_skills_catalog_request(request, user.into()),
        )? {
            LocalDaemonResponse::SkillsV2 { result } => Ok(result),
            other => panic!("a Skills request answers Skills: {other:?}"),
        }
    }

    fn catalog(
        &self,
        agent_id: Option<&str>,
        user: &str,
    ) -> (
        Vec<NativeSkillEntry>,
        Vec<crate::skill::v2::SkillProjection>,
    ) {
        let catalog = self.request(
            SkillsV2Request::Catalog {
                project_id: Some(self.project_id.clone()),
                agent_id: agent_id.map(Into::into),
            },
            user,
        );
        let Ok(SkillsV2Response::Catalog { native, skills, .. }) = catalog else {
            panic!("the catalog request answers a catalog: {catalog:?}");
        };
        (native, skills)
    }

    fn details(&self, selection: &SkillSelection, user: &str) -> Result<String, DaemonError> {
        match self.request(
            SkillsV2Request::NativeDetails {
                agent_id: None,
                selection: selection.clone(),
            },
            user,
        )? {
            SkillsV2Response::NativeDetails { details } => Ok(details.body.unwrap_or_default()),
            other => panic!("Details answers the provider skill: {other:?}"),
        }
    }

    fn install_chariox_skill(&self, skill_md: &str) {
        use crate::skill::v2::{
            SkillCatalogAction, SkillCatalogMutation, SkillCollisionAction, SkillInstallSource,
            SkillScope,
        };
        let service = self.state.skills_catalog_service().unwrap();
        let mutation = SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: service.storage.snapshot().unwrap().revision,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content {
                    skill_md: skill_md.into(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        };
        let prepared = service.prepare(&mutation).unwrap();
        service.publish_authorized(&mutation, &prepared).unwrap();
    }
}

fn listed(native: &[NativeSkillEntry]) -> Vec<(String, String, String)> {
    native
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
        .collect()
}

/// A Project no provider ever ran in lists the skills of an installed provider: the
/// Catalog with no focused agent carries them, and Details reads one with no agent.
#[test]
fn a_project_lists_an_installed_providers_skills_with_no_run() {
    crate::test_support::isolated_env_test!();
    let project = Installed::new("default");
    let (native, _) = project.catalog(None, OWNER);
    let daemon_id = project.state.owned.config_projection.snapshot().daemon_id;
    assert_eq!(
        listed(&native),
        [(
            "release-notes".to_string(),
            "claude".to_string(),
            installed_execution_id("claude", &daemon_id, &project.project_id),
        )]
    );
    assert!(project
        .details(&native[0].selection, OWNER)
        .unwrap()
        .contains("List the changes."));
}

/// The host's own provider accounts are the owner's: a Room member with no account of
/// their own on this kernel sees none of their skills, in the list or in Details.
#[test]
fn a_room_member_does_not_see_the_owners_account_skills() {
    crate::test_support::isolated_env_test!();
    let project = Installed::new("default");
    let mut room = project
        .state
        .owned
        .session_store
        .get_session(&project.session_id)
        .unwrap();
    room.add_member(
        "guest",
        Some(OWNER.into()),
        crate::session::CollaborationLevel::Full,
    );
    project
        .state
        .owned
        .session_store
        .write()
        .restore_session(room);
    let (owner_native, _) = project.catalog(None, OWNER);
    assert_eq!(owner_native.len(), 1, "the owner sees the account skill");

    let (guest_native, _) = project.catalog(None, "guest");
    assert!(guest_native.is_empty(), "{:?}", listed(&guest_native));
    let refused = project
        .details(&owner_native[0].selection, "guest")
        .unwrap_err();
    assert!(!refused.to_string().contains("List the changes."));
}

/// A focused agent whose provider has not started yet has no run to list its skills,
/// so its installed provider lists them.
#[test]
fn a_focused_agent_that_never_started_lists_its_installed_providers_skills() {
    crate::test_support::isolated_env_test!();
    let project = Installed::new("claude");
    let (native, _) = project.catalog(Some(&project.agent_id), OWNER);
    assert_eq!(
        listed(&native)
            .into_iter()
            .map(|(name, provider, _)| (name, provider))
            .collect::<Vec<_>>(),
        [("release-notes".to_string(), "claude".to_string())]
    );
}

/// An installed provider's listing is shown, but only the focused run decides how a
/// Chariox skill reaches the turn: a same-named account skill does not take it over.
#[test]
fn an_installed_providers_skill_does_not_change_how_a_chariox_skill_is_delivered() {
    crate::test_support::isolated_env_test!();
    let project = Installed::new("default");
    project.install_chariox_skill(
        "---\nname: release-notes\ndescription: Write release notes\n---\nList the changes.\n",
    );
    let (native, skills) = project.catalog(None, OWNER);
    assert_eq!(
        native.len(),
        1,
        "the Providers tab still lists the account skill"
    );
    let row = skills
        .iter()
        .find(|row| row.skill.name == "release-notes")
        .unwrap();
    assert_eq!(row.delivery, crate::skill::v2::SkillDelivery::Chariox);
    assert_eq!(row.also_in_provider, None);
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
