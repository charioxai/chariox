//! MP-08 / MP-10 / MP-11: provider seam regressions, not SV acceptance.
use super::*;
use std::fs;
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("skillsp-inventory-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn skill(&self, root: &str, name: &str) -> PathBuf {
        let path = self.0.join(root).join(name).join("SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: Review source safely\n---\nInspect source.\n"),
        )
        .unwrap();
        path
    }
    fn binding(&self, provider: &str) -> SkillInventoryBinding {
        SkillInventoryBinding {
            provider: provider.into(),
            provider_version: "pinned-test".into(),
            account_profile: "selected-account".into(),
            execution_id: "execution-1".into(),
            cwd: self.0.clone(),
            environment: BTreeMap::from([(
                "HOME".into(),
                self.0.join("home").display().to_string(),
            )]),
            project_id: Some("project-1".into()),
            catalog_revision: 1,
            source_revision: 1,
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn candidate(path: PathBuf) -> NativeSkillCandidate {
    NativeSkillCandidate {
        name: "review".into(),
        description: "Review source safely".into(),
        skill_file: path,
        exposure: NativeSkillExposure::Confirmed,
        supports_load_ack: true,
        observed_body_digest: None,
        discovery_error: None,
    }
}
#[test]
fn full_content_and_executable_modes_change_inventory_revision() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let binding = temp.binding("codex");
    let a = NativeSkillInventory::from_candidates(&binding, vec![candidate(path.clone())]).unwrap();
    fs::write(
        path.parent().unwrap().join("reference.md"),
        "different bundled asset",
    )
    .unwrap();
    let b = NativeSkillInventory::from_candidates(&binding, vec![candidate(path.clone())]).unwrap();
    assert_ne!(a.entries[0].digest, b.entries[0].digest);
    assert_ne!(a.revision, b.revision);
    assert!(b.native_path(&a.entries[0].selection).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path.parent().unwrap().join("reference.md"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let c = NativeSkillInventory::from_candidates(&binding, vec![candidate(path)]).unwrap();
        assert_ne!(b.entries[0].digest, c.entries[0].digest);
    }
}
#[test]
fn missing_package_and_directory_candidates_never_confirm_exposure() {
    let temp = Scratch::new();
    let mut c = candidate(temp.0.join("absent/SKILL.md"));
    let binding = temp.binding("claude");
    let result = NativeSkillInventory::from_candidates(&binding, vec![c.clone()]).unwrap();
    assert_eq!(result.entries[0].exposure, NativeSkillExposure::Unverified);
    assert!(!result.entries[0].complete_manifest);
    c.skill_file = temp.skill("skills", "review");
    c.exposure = NativeSkillExposure::Unverified;
    let result = NativeSkillInventory::from_candidates(&binding, vec![c]).unwrap();
    assert_eq!(result.entries[0].exposure, NativeSkillExposure::Unverified);
    assert!(result.entries[0].complete_manifest);
    assert!(result.native_path(&result.entries[0].selection).is_err());
}
#[test]
fn cache_reloads_for_all_bound_discovery_inputs_and_explicit_invalidation() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let mut binding = temp.binding("codex");
    let mut cache = NativeSkillInventoryCache::default();
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            Ok(vec![candidate(path.clone())])
        })
        .unwrap();
    cache
        .inventory(&binding, false, |_| panic!("must use fresh cached listing"))
        .unwrap();
    binding.account_profile = "other-account".into();
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            Ok(vec![])
        })
        .unwrap();
    binding.catalog_revision += 1;
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            Ok(vec![])
        })
        .unwrap();
    binding.source_revision += 1;
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            Ok(vec![])
        })
        .unwrap();
    cache.invalidate();
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            Ok(vec![])
        })
        .unwrap();
    assert!(cache
        .inventory(&binding, true, |_| Err(failure("offline")))
        .is_err());
    cache.inventory(&binding, false, |_| Ok(vec![])).unwrap();
}
#[test]
fn codex_respects_actual_cwd_disabled_state_and_discovery_errors() {
    let response = serde_json::json!({"data":[{"cwd":"/actual", "skills":[
        {"name":"review","description":"Review", "path":"/skills/review/SKILL.md","enabled":false},
        {"name":"other","description":"Other", "path":"/skills/other/SKILL.md","enabled":true}], "errors":[{"message":"discovery failed"}]}]});
    let result = codex_skill_candidates(&response, std::path::Path::new("/actual")).unwrap();
    assert_eq!(result[0].exposure, NativeSkillExposure::Disabled);
    assert_eq!(result[1].exposure, NativeSkillExposure::Unverified);
    assert!(codex_skill_candidates(&response, std::path::Path::new("/other")).is_err());
}
#[test]
fn claude_scoped_profile_additional_and_plugins_confirm_exposure() {
    let temp = Scratch::new();
    let mut binding = temp.binding("claude");
    binding.environment.insert(
        "CLAUDE_CONFIG_DIR".into(),
        temp.0.join("selected-profile").display().to_string(),
    );
    temp.skill("selected-profile/skills", "profile");
    temp.skill("home/.claude/skills", "ambient");
    temp.skill(".claude/skills", "project");
    temp.skill("extra/.claude/skills", "extra");
    temp.skill("plugin/skills", "plugin");
    let candidates = filesystem_skill_candidates(
        &binding,
        &temp.0,
        &[temp.0.join("extra")],
        &[temp.0.join("plugin/skills")],
        &[],
    )
    .unwrap();
    let names: BTreeMap<_, _> = candidates
        .iter()
        .map(|c| (c.name.as_str(), &c.exposure))
        .collect();
    assert_eq!(names.len(), 4);
    assert!(!names.contains_key("ambient"));
    assert!(names
        .values()
        .all(|state| **state == NativeSkillExposure::Confirmed));
}
#[test]
fn claude_settings_permission_deny_disables_the_native_skill() {
    let temp = Scratch::new();
    let mut binding = temp.binding("claude");
    binding.environment.insert(
        "CLAUDE_CONFIG_DIR".into(),
        temp.0.join("selected-profile").display().to_string(),
    );
    temp.skill("selected-profile/skills", "account-denied");
    temp.skill(".claude/skills", "project-denied");
    temp.skill(".claude/skills", "local-denied");
    temp.skill(".claude/skills", "kept");
    fs::write(
        temp.0.join("selected-profile/settings.json"),
        r#"{"permissions":{"deny":["Bash(rm *)","Skill(account-denied)"]},"env":{"SECRET":"x"}}"#,
    )
    .unwrap();
    fs::write(
        temp.0.join(".claude/settings.json"),
        r#"{"permissions":{"deny":["Skill(project-denied *)"]}}"#,
    )
    .unwrap();
    fs::write(
        temp.0.join(".claude/settings.local.json"),
        r#"{"permissions":{"deny":["Skill(local-denied)"]}}"#,
    )
    .unwrap();
    let candidates = filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[]).unwrap();
    let inventory = NativeSkillInventory::from_candidates(&binding, candidates).unwrap();
    let exposure = |name: &str| {
        inventory
            .entries
            .iter()
            .find(|entry| matches!(&entry.selection, SkillSelection::Native { name: n, .. } if n == name))
            .unwrap()
            .exposure
            .clone()
    };
    assert_eq!(exposure("account-denied"), NativeSkillExposure::Disabled);
    assert_eq!(exposure("project-denied"), NativeSkillExposure::Disabled);
    assert_eq!(exposure("local-denied"), NativeSkillExposure::Disabled);
    assert_eq!(exposure("kept"), NativeSkillExposure::Confirmed);
}
#[test]
fn opencode_scoped_xdg_compatibility_and_configured_roots_are_candidates() {
    let temp = Scratch::new();
    let mut binding = temp.binding("opencode");
    binding.environment.insert(
        "XDG_CONFIG_HOME".into(),
        temp.0.join("xdg").display().to_string(),
    );
    binding.environment.insert(
        "OPENCODE_CONFIG_DIR".into(),
        temp.0.join("scoped").display().to_string(),
    );
    temp.skill("xdg/opencode/skills", "xdg");
    temp.skill("scoped/skill", "scoped");
    temp.skill(".claude/skills", "claude");
    temp.skill(".agents/skills", "agents");
    temp.skill("configured", "configured");
    let candidates =
        filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[temp.0.join("configured")])
            .unwrap();
    assert_eq!(candidates.len(), 5);
    binding
        .environment
        .insert("OPENCODE_DISABLE_EXTERNAL_SKILLS".into(), "true".into());
    assert_eq!(
        filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[temp.0.join("configured")])
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn inventory_binding_rejects_ambient_or_secret_environment() {
    let temp = Scratch::new();
    let mut binding = temp.binding("codex");
    binding.cwd = "relative".into();
    assert!(binding.validate().is_err());
    binding.cwd = temp.0.clone();
    binding
        .environment
        .insert("API_KEY".into(), "synthetic-never-used".into());
    assert!(binding.validate().is_err());
}
#[test]
fn native_invocation_rechecks_the_complete_package_at_delivery() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let inventory = NativeSkillInventory::from_candidates(
        &temp.binding("codex"),
        vec![candidate(path.clone())],
    )
    .unwrap();
    fs::write(
        path.parent().unwrap().join("asset.txt"),
        "changed after listing",
    )
    .unwrap();
    assert!(
        inventory
            .native_path(&inventory.entries[0].selection)
            .is_err(),
        "stale paths must never load a changed native package"
    );
}
#[test]
fn codex_native_implicit_invocation_policy_is_part_of_visibility() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    fs::create_dir_all(path.parent().unwrap().join("agents")).unwrap();
    fs::write(
        path.parent().unwrap().join("agents/openai.yaml"),
        "policy:\n  allow_implicit_invocation: false\n",
    )
    .unwrap();
    let inventory =
        NativeSkillInventory::from_candidates(&temp.binding("codex"), vec![candidate(path)])
            .unwrap();
    assert_eq!(inventory.entries[0].default_state, SkillState::Manual);
}
#[test]
fn cached_source_content_change_forces_official_reload_before_reconfirming() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let binding = temp.binding("codex");
    let mut cache = NativeSkillInventoryCache::default();
    cache
        .inventory(&binding, false, |_| Ok(vec![candidate(path.clone())]))
        .unwrap();
    fs::write(path.parent().unwrap().join("new-asset"), "new bytes").unwrap();
    let mut reloaded = false;
    cache
        .inventory(&binding, false, |force| {
            assert!(force);
            reloaded = true;
            Ok(vec![candidate(path)])
        })
        .unwrap();
    assert!(reloaded);
}
#[test]
fn failed_refresh_revokes_confirmed_authority_without_suppressing_chariox() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let binding = temp.binding("codex");
    let mut cache = NativeSkillInventoryCache::default();
    let before = cache
        .inventory(&binding, false, |_| Ok(vec![candidate(path)]))
        .unwrap();
    let unavailable = cache
        .inventory(&binding, true, |_| Err(failure("offline")))
        .unwrap();
    assert_eq!(
        unavailable.entries[0].exposure,
        NativeSkillExposure::Unverified
    );
    assert_ne!(before.revision, unavailable.revision);
    assert!(unavailable
        .native_path(&before.entries[0].selection)
        .is_err());
    assert!(cache.cached.is_none());
}
#[test]
fn opencode_cannot_dedupe_a_new_disk_body_against_its_stale_server_body() {
    let temp = Scratch::new();
    let path = temp.skill("skills", "review");
    let mut c = candidate(path);
    c.observed_body_digest = Some(hash(b"body retained from a previous turn"));
    let inventory =
        NativeSkillInventory::from_candidates(&temp.binding("opencode"), vec![c]).unwrap();
    assert_eq!(
        inventory.entries[0].exposure,
        NativeSkillExposure::Unverified
    );
    assert!(!inventory.entries[0].complete_manifest);
}
#[test]
fn launch_environment_capture_keeps_only_resolved_discovery_inputs() {
    let env = capture_launch_skill_environment(
        &BTreeMap::from([
            ("CODEX_HOME".into(), "/selected-profile".into()),
            ("HOME".into(), "/execution-home".into()),
            ("OPENAI_API_KEY".into(), "synthetic-not-a-credential".into()),
        ]),
        &["CLAUDE_CONFIG_DIR".into()],
    );
    assert_eq!(env.get("CODEX_HOME").unwrap(), "/selected-profile");
    assert_eq!(env.get("HOME").unwrap(), "/execution-home");
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("CLAUDE_CONFIG_DIR"));
}
#[test]
fn opencode_configured_paths_include_nested_and_standalone_packages() {
    let temp = Scratch::new();
    let binding = temp.binding("opencode");
    temp.skill("configured/nested/group", "review");
    let path = temp.skill("standalone", "alone");
    let candidates = filesystem_skill_candidates(
        &binding,
        &temp.0,
        &[],
        &[],
        &[temp.0.join("configured"), path.parent().unwrap().into()],
    )
    .unwrap();
    assert_eq!(candidates.len(), 2);
}
#[cfg(unix)]
#[test]
fn candidate_enumeration_does_not_follow_skill_file_symlinks() {
    let temp = Scratch::new();
    let binding = temp.binding("claude");
    let source = temp.skill("outside", "review");
    let destination = temp.0.join("home/.claude/skills/review/SKILL.md");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(source, destination).unwrap();
    let candidates = filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[]).unwrap();
    assert!(candidates.is_empty());
}
#[test]
fn runtime_inventory_environment_is_profile_bound_and_never_a_serialized_input() {
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    let run = RuntimeProviderRun::new(
        "run",
        &LaunchProviderRequest::new("session", "codex", "codex", "selected", "default"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: BTreeMap::from([("CODEX_HOME".into(), "/selected-profile".into())]),
            pty_env_remove: vec![],
            working_directory: Some("/actual-cwd".into()),
            structured_endpoint: Some("ws://127.0.0.1:1".into()),
        },
    );
    assert_eq!(
        run.native_skill_discovery_environment()
            .get("CODEX_HOME")
            .unwrap(),
        "/selected-profile"
    );
    let mut value = serde_json::to_value(&run).unwrap();
    assert!(value.get("native_skill_discovery_environment").is_none());
    value["native_skill_discovery_environment"] =
        serde_json::json!({"CODEX_HOME":"/forged-profile"});
    let decoded: RuntimeProviderRun = serde_json::from_value(value).unwrap();
    assert!(decoded.native_skill_discovery_environment().is_empty());
}
#[test]
fn native_details_return_execution_manifest_and_provenance_without_installing() {
    let temp = Scratch::new();
    let path = temp.skill("native", "review");
    fs::write(
        path.parent().unwrap().join("reference.md"),
        "reference content",
    )
    .unwrap();
    let binding = temp.binding("codex");
    let inventory =
        NativeSkillInventory::from_candidates(&binding, vec![candidate(path.clone())]).unwrap();
    let details = inventory.details(&inventory.entries[0].selection).unwrap();
    assert_eq!(details.provenance.provider_version, "pinned-test");
    assert_eq!(details.provenance.account_profile, "selected-account");
    assert_eq!(details.provenance.execution_id, "execution-1");
    assert_eq!(details.provenance.cwd, temp.0);
    assert_eq!(details.root.as_deref(), path.parent());
    assert_eq!(details.files.len(), 2);
    assert!(details
        .files
        .iter()
        .any(|file| file.path == "reference.md" && file.bytes == 17));
    assert!(details.body.unwrap().contains("Inspect source."));
    assert_eq!(details.manifest_digest, inventory.entries[0].digest);
    assert!(!temp.0.join("skills/packages").exists());
    fs::write(path.parent().unwrap().join("reference.md"), "changed").unwrap();
    assert!(inventory.details(&inventory.entries[0].selection).is_err());
}
#[test]
fn unverified_native_details_do_not_promote_exposure_or_accept_foreign_selections() {
    let temp = Scratch::new();
    let mut c = candidate(temp.skill("native", "review"));
    c.exposure = NativeSkillExposure::Unverified;
    let inventory =
        NativeSkillInventory::from_candidates(&temp.binding("claude"), vec![c]).unwrap();
    let selection = &inventory.entries[0].selection;
    assert_eq!(
        inventory.details(selection).unwrap().entry.exposure,
        NativeSkillExposure::Unverified
    );
    assert!(inventory.native_path(selection).is_err());
    let mut foreign = selection.clone();
    if let SkillSelection::Native {
        account_profile, ..
    } = &mut foreign
    {
        *account_profile = "other".into();
    }
    assert!(inventory.details(&foreign).is_err());
}

#[test]
fn claude_directory_selection_uses_native_slash_adapter_and_rechecks_digest() {
    let temp = Scratch::new();
    let binding = temp.binding("claude");
    let path = temp.skill(".claude/skills", "review");
    let candidates = filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[]).unwrap();
    let inventory = NativeSkillInventory::from_candidates(&binding, candidates).unwrap();
    let selection = inventory.entries[0].selection.clone();
    let mut context = SkillTurnContext {
        catalog_revision: 1,
        context_generation: "test".into(),
        metadata: String::new(),
        metadata_tokens_upper_bound: 0,
        metadata_budget_tokens: 100,
        omitted_auto_count: 0,
        native_selections: vec![],
        loaded: vec![],
    };
    context.native_selections.push(selection.clone());
    let delivery = crate::provider::skill_delivery::prepare_skill_delivery(
        "claude",
        "$review inspect source",
        &context,
        &inventory,
        &[selection.clone()],
    )
    .expect("verified directory selection must reach Claude's native adapter");
    assert_eq!(delivery.visible_prompt, "/review inspect source");
    assert!(delivery.hidden_context.is_empty());
    assert!(delivery.codex_input.is_empty());
    assert_eq!(delivery.requires_native_load_ack, vec![selection]);
    fs::write(
        path,
        "---\nname: review\ndescription: Changed\n---\nChanged body.\n",
    )
    .unwrap();
    assert!(crate::provider::skill_delivery::prepare_skill_delivery(
        "claude",
        "$review inspect source",
        &context,
        &inventory,
        &[],
    )
    .is_err());
}

#[test]
fn claude_account_skill_shadows_same_name_project_skill() {
    let temp = Scratch::new();
    let binding = temp.binding("claude");
    let account = temp.skill("home/.claude/skills", "review");
    let project = temp.skill(".claude/skills", "review");
    fs::write(
        &project,
        "---\nname: review\ndescription: Project review\n---\nProject body.\n",
    )
    .unwrap();
    let candidates = filesystem_skill_candidates(&binding, &temp.0, &[], &[], &[]).unwrap();
    let inventory = NativeSkillInventory::from_candidates(&binding, candidates).unwrap();
    assert_eq!(
        inventory.entries.len(),
        1,
        "shadowed namesake must not be offered"
    );
    assert_eq!(
        inventory
            .native_path(&inventory.entries[0].selection)
            .unwrap(),
        account,
        "the account skill is the harness-effective package"
    );
}
