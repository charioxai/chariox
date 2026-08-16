use super::*;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-skill-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

#[test]
fn registry_and_remote_materialization_roots_can_be_isolated_for_managed_slice_runtime() {
    let _guard = crate::env_lock::lock();
    let isolation_root = temp_root("managed-slice-isolation");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);

    let project_root = CharioxSkillRegistry::project_root("/workspace");
    let user_root = CharioxSkillRegistry::user_root().expect("user root should resolve");
    let remote_root = remote_skill_materialization_base("/workspace");

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    let _ = fs::remove_dir_all(&isolation_root);

    assert!(project_root.starts_with(isolation_root.join("project")));
    assert!(project_root.ends_with("skills"));
    assert_eq!(user_root, isolation_root.join("user").join("skills"));
    assert_eq!(remote_root, isolation_root.join("remote").join("skills"));
}

#[test]
fn detects_explicit_skill_requests() {
    assert!(prompt_explicitly_requests_skill(
        "Use browser-qa to validate this flow",
        "browser-qa"
    ));
    assert!(prompt_explicitly_requests_skill(
        "Please apply @release_check",
        "release_check"
    ));
    assert!(prompt_explicitly_requests_skill(
        "Run the `security-review` skill",
        "security-review"
    ));
    assert!(!prompt_explicitly_requests_skill(
        "This browser-qa-extra text is another skill",
        "browser-qa"
    ));
}

#[test]
fn granted_skill_prompt_context_uses_the_prompt_catalog_template() {
    let _guard = crate::env_lock::lock();
    let home = temp_root("prompt-context-home");
    let skill_dir = home.join(".chariox").join("skills").join("browser-qa");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA workflow\nshort-description: QA\n---\nRun the browser checklist.\n",
    )
    .unwrap();
    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);

    let context = format_granted_skill_prompt_context(
        "agent-ref",
        &["browser-qa".to_string()],
        "/workspace",
        "Use browser-qa to validate this flow",
    )
    .expect("skill prompt context should render");

    match previous_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    let _ = fs::remove_dir_all(home);

    assert!(context.contains("<skill-context-instructions>"));
    assert!(context.contains("Available Chariox skills for this agent:"));
    assert!(context.contains("- `browser-qa`: QA"));
    assert!(context.contains("<chariox_skill name=\"browser-qa\">"));
    assert!(context.contains("Run the browser checklist."));
}

#[test]
fn parses_codex_style_skill_metadata() {
    let root = temp_root("parse");
    let skill_dir = root.join("browser-qa");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA workflow\nshort-description: QA\n---\nUse the browser.\n",
    )
    .unwrap();

    let registry = CharioxSkillRegistry::new(vec![root.clone()]);
    let skills = registry.list().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "browser-qa");
    assert_eq!(skills[0].description, "Browser QA workflow");
    assert_eq!(skills[0].short_description.as_deref(), Some("QA"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn install_copies_skill_directory_to_primary_root() {
    let source_root = temp_root("install-source");
    let registry_root = temp_root("install-registry");
    let skill_dir = source_root.join("browser-qa");
    fs::create_dir_all(skill_dir.join("assets")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA workflow\n---\nUse the browser.\n",
    )
    .unwrap();
    fs::write(skill_dir.join("assets").join("prompt.txt"), "qa checklist").unwrap();

    let registry = CharioxSkillRegistry::new(vec![registry_root.clone()]);
    let (metadata, destination) = registry.install_from_path(&skill_dir).unwrap();

    assert_eq!(metadata.name, "browser-qa");
    assert_eq!(destination, registry_root.join("browser-qa"));
    assert_eq!(
        fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
        "qa checklist"
    );
    assert_eq!(registry.get("browser-qa").unwrap(), Some(metadata));

    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(registry_root);
}

#[test]
fn update_replaces_and_uninstall_removes_existing_skill() {
    let source_root = temp_root("update-source");
    let registry_root = temp_root("update-registry");
    let original_dir = source_root.join("original");
    let updated_dir = source_root.join("updated");
    fs::create_dir_all(&original_dir).unwrap();
    fs::write(
        original_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Old QA\n---\nOld body.\n",
    )
    .unwrap();
    fs::write(original_dir.join("old.txt"), "old").unwrap();
    fs::create_dir_all(updated_dir.join("assets")).unwrap();
    fs::write(
        updated_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: New QA\n---\nNew body.\n",
    )
    .unwrap();
    fs::write(updated_dir.join("assets").join("new.txt"), "new").unwrap();

    let registry = CharioxSkillRegistry::new(vec![registry_root.clone()]);
    registry.install_from_path(&original_dir).unwrap();
    let (updated, destination) = registry.update_from_path(&updated_dir).unwrap();
    assert_eq!(updated.description, "New QA");
    assert_eq!(destination, registry_root.join("browser-qa"));
    assert!(!destination.join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("assets").join("new.txt")).unwrap(),
        "new"
    );

    let (removed, removed_path) = registry.uninstall("browser-qa").unwrap();
    assert_eq!(removed.name, "browser-qa");
    assert_eq!(removed_path, registry_root.join("browser-qa"));
    assert_eq!(registry.get("browser-qa").unwrap(), None);
    assert!(!removed_path.exists());

    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(registry_root);
}

#[test]
fn upsert_from_path_replaces_existing_skill_directory_with_assets() {
    let source_root = temp_root("upsert-path-source");
    let registry_root = temp_root("upsert-path-registry");
    let original_dir = source_root.join("original");
    let updated_dir = source_root.join("updated");
    fs::create_dir_all(&original_dir).unwrap();
    fs::write(
        original_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Old QA\n---\nOld body.\n",
    )
    .unwrap();
    fs::write(original_dir.join("old.txt"), "old").unwrap();
    fs::create_dir_all(updated_dir.join("assets")).unwrap();
    fs::write(
        updated_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: New QA\n---\nNew body.\n",
    )
    .unwrap();
    fs::write(updated_dir.join("assets").join("prompt.txt"), "new prompt").unwrap();

    let registry = CharioxSkillRegistry::new(vec![registry_root.clone()]);
    registry.upsert_from_path(&original_dir).unwrap();
    let (updated, destination) = registry.upsert_from_path(&updated_dir).unwrap();

    assert_eq!(updated.description, "New QA");
    assert_eq!(destination, registry_root.join("browser-qa"));
    assert!(!destination.join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
        "new prompt"
    );

    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(registry_root);
}

#[test]
fn imports_codex_and_opencode_skill_roots() {
    let workspace = temp_root("import-workspace");
    let registry_root = temp_root("import-registry");
    let codex_skill = workspace.join(".codex").join("skills").join("codex-qa");
    let opencode_skill = workspace
        .join(".opencode")
        .join("skills")
        .join("opencode-qa");
    let claude_skill = workspace.join(".claude").join("skills").join("claude-qa");
    fs::create_dir_all(codex_skill.join("assets")).unwrap();
    fs::create_dir_all(&opencode_skill).unwrap();
    fs::create_dir_all(&claude_skill).unwrap();
    fs::write(
        codex_skill.join("SKILL.md"),
        "---\nname: codex-qa\ndescription: Codex QA\n---\nUse Codex QA.\n",
    )
    .unwrap();
    fs::write(codex_skill.join("assets").join("checklist.txt"), "check").unwrap();
    fs::write(
        opencode_skill.join("SKILL.md"),
        "---\nname: opencode-qa\ndescription: OpenCode QA\n---\nUse OpenCode QA.\n",
    )
    .unwrap();
    fs::write(
        claude_skill.join("SKILL.md"),
        "---\nname: claude-qa\ndescription: Claude QA\n---\nUse Claude QA.\n",
    )
    .unwrap();

    let registry = CharioxSkillRegistry::new(vec![registry_root.clone()]);
    let codex = import_codex_skills(&registry, &workspace, Some("codex-qa")).unwrap();
    assert_eq!(codex.imported.len(), 1);
    assert_eq!(codex.imported[0].name, "codex-qa");
    assert_eq!(
        fs::read_to_string(
            registry_root
                .join("codex-qa")
                .join("assets")
                .join("checklist.txt")
        )
        .unwrap(),
        "check"
    );

    let opencode = import_opencode_skills(&registry, &workspace, Some("opencode-qa")).unwrap();
    assert_eq!(opencode.imported.len(), 1);
    assert_eq!(opencode.imported[0].name, "opencode-qa");

    let claude = import_claude_skills(&registry, &workspace, Some("claude-qa")).unwrap();
    assert_eq!(claude.imported.len(), 1);
    assert_eq!(claude.imported[0].name, "claude-qa");

    let duplicate = import_codex_skills(&registry, &workspace, Some("codex-qa")).unwrap();
    assert!(duplicate.imported.is_empty());
    assert_eq!(duplicate.skipped[0].name, "codex-qa");
    assert!(duplicate.skipped[0].reason.contains("already installed"));

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(registry_root);
}

#[test]
fn packages_and_materializes_complete_skill_directory() {
    let source_root = temp_root("package-source");
    let materialized_root = temp_root("package-materialized");
    let skill_dir = source_root.join("browser-qa");
    fs::create_dir_all(skill_dir.join("assets").join("nested")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA workflow\n---\nUse assets/prompt.txt.\n",
    )
    .unwrap();
    fs::write(skill_dir.join("assets").join("prompt.txt"), "qa checklist").unwrap();
    fs::write(
        skill_dir.join("assets").join("nested").join("fixture.json"),
        "{\"ok\":true}",
    )
    .unwrap();

    let package = package_skill_directory(&skill_dir).unwrap();
    assert_eq!(package.metadata.name, "browser-qa");
    assert!(package.files.iter().any(|file| file.path == "SKILL.md"));
    assert!(package
        .files
        .iter()
        .any(|file| file.path == "assets/prompt.txt"));
    assert!(package
        .files
        .iter()
        .any(|file| file.path == "assets/nested/fixture.json"));

    let destination = materialize_skill_package(&materialized_root, &package).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("SKILL.md")).unwrap(),
        fs::read_to_string(skill_dir.join("SKILL.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
        "qa checklist"
    );
    assert_eq!(
        package_skill_directory(&destination).unwrap().version_hash,
        package.version_hash
    );

    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(materialized_root);
}

#[test]
fn package_skips_symlinks_and_ignored_directories() {
    let root = temp_root("package-symlink");
    let skill_dir = root.join("safe");
    fs::create_dir_all(skill_dir.join(".git")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: safe\ndescription: Safe skill\n---\nBody\n",
    )
    .unwrap();
    fs::write(skill_dir.join(".git").join("config"), "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", skill_dir.join("passwd-link")).unwrap();

    let package = package_skill_directory(&skill_dir).unwrap();
    assert!(package
        .files
        .iter()
        .all(|file| !file.path.starts_with(".git")));
    assert!(package.files.iter().all(|file| file.path != "passwd-link"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_skill_without_description() {
    let root = temp_root("bad");
    let skill_dir = root.join("bad-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: bad-skill\n---\nBody\n",
    )
    .unwrap();

    let err = parse_skill_metadata(&skill_dir.join("SKILL.md")).unwrap_err();
    assert!(format!("{err}").contains("description"));

    let _ = fs::remove_dir_all(root);
}
