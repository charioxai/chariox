//! MP-08 / MP-10 / MP-11: actual owned-store startup, files, journal and old writer.
use super::*;
use crate::durable_state::DurableKernelStateStore;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skillsk-startup-{:032x}", rand::random::<u128>()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn config(&self, name: &str) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.user_config_path = self.home().join("config.toml");
        config.user_config.state.path =
            Some(self.0.join(name).join("state.db").display().to_string());
        config
    }
    fn old_package(&self) {
        let path = self.home().join("skills/review");
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nOld guidance.\n",
        )
        .unwrap();
    }
    fn seed_old(&self, config: &DaemonConfig) {
        let store = DurableKernelStateStore::open(config.durable_state_path()).unwrap();
        store.append_event("session.updated",Some("session".into()),json!({
            "agents":[{"extension_grants":[{"kind":"skill","name":"review"},{"kind":"mcp","name":"safe"}]}],
            "workflows":[{"id":"wf","prompt":"Use $review then explain $total"}],
            "queued_prompts":[{"id":"old","prompt":"Use $review"},{"id":"ordinary","prompt":"Explain $total"}]
        })).unwrap();
    }
    fn artifact(&self, name: &str, prompt: &str) -> PathBuf {
        use crate::workflow_code::*;
        let root = self.home().join("workflow-code");
        let definition:WorkflowCodeDefinition=serde_json::from_value(json!({
            "workflow":{"alias":name,"prompt":prompt},
            "nodes":[{"handle":"reviewer","agent":{"kind":"create","provider":"claude"},
                "instructions":"Preserve ordinary text and $total", "extensions":[{"kind":"mcp","name":"safe"}]}]
        })).unwrap();
        let validation =
            definition.validate_with_limits(&crate::config::WorkflowCodeLimitsConfig::default());
        WorkflowCodeArtifactRegistry::new(vec![root.clone()])
            .save(
                name,
                WorkflowCodeLanguage::JavaScript,
                format!(
                    "export default function(w) {{ return w.workflow({{prompt:{}}}); }}",
                    serde_json::to_string(prompt).unwrap()
                ),
                definition,
                validation,
                WorkflowCodeArtifactActor::new("owner", None),
                WorkflowCodeArtifactHistoryAction::Created,
            )
            .unwrap();
        root.join(format!("{name}.json"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn skills_v2_owned_startup_retires_packages_artifacts_and_arms_the_live_writer() {
    let f = Fixture::new();
    let config = f.config("owner");
    f.old_package();
    f.seed_old(&config);
    let artifact = f.artifact("review-workflow", "Use $review then explain $total");
    let old = Connection::open(config.durable_state_path()).unwrap();
    let store = DurableKernelStateStore::open_kernel_owned(&config).unwrap();
    assert!(store.owns_skill_catalog());
    assert!(store.skills_v2_reset_complete().unwrap());
    assert!(!f.home().join("skills/review").exists());
    let rows = store
        .load_subject_events_by_kind("session", "session.updated", 1)
        .unwrap();
    let state = &rows.last().unwrap().payload;
    assert_eq!(
        state["agents"][0]["extension_grants"],
        json!([{"kind":"mcp","name":"safe"}])
    );
    assert_eq!(state["workflows"][0]["prompt"], "Use  then explain $total");
    assert_eq!(
        state["queued_prompts"],
        json!([{"id":"ordinary","prompt":"Explain $total"}])
    );
    let stored: Value = serde_json::from_slice(&fs::read(&artifact).unwrap()).unwrap();
    assert_eq!(
        stored["definition"]["workflow"]["prompt"],
        "Use  then explain $total"
    );
    assert!(!stored["source"].as_str().unwrap().contains("$review"));
    assert_eq!(
        stored["definition"]["nodes"][0]["extensions"],
        json!([{"kind":"mcp","name":"safe"}])
    );
    assert!(old.execute("DELETE FROM durable_state_events", []).is_err());
    store
        .append_event("skills.v2.test", None, json!({"upgraded":true}))
        .unwrap();
}

#[test]
fn skills_v2_completed_startup_does_not_rewrite_new_same_name_workflows() {
    let f = Fixture::new();
    let config = f.config("owner");
    f.old_package();
    f.seed_old(&config);
    let store = DurableKernelStateStore::open_kernel_owned(&config).unwrap();
    drop(store);
    let artifact = f.artifact(
        "after-upgrade",
        "Use $review for this newly authored workflow",
    );
    let before = fs::read(&artifact).unwrap();
    let package = f.home().join("skills/packages/new-id").join("a".repeat(64));
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("SKILL.md"), "new v2 package bytes").unwrap();
    let store = DurableKernelStateStore::open_kernel_owned(&config).unwrap();
    assert!(store.skills_v2_reset_complete().unwrap());
    assert_eq!(fs::read(artifact).unwrap(), before);
    assert_eq!(
        fs::read_to_string(package.join("SKILL.md")).unwrap(),
        "new v2 package bytes"
    );
}

#[test]
fn skills_v2_second_journal_retires_own_state_without_rewriting_the_home_owners_new_artifacts() {
    let f = Fixture::new();
    let a = f.config("owner");
    f.old_package();
    f.seed_old(&a);
    let owner = DurableKernelStateStore::open_kernel_owned(&a).unwrap();
    let artifact = f.artifact(
        "new-workflow",
        "Use $review for this newly authored workflow",
    );
    let before = fs::read(&artifact).unwrap();
    let b = f.config("other");
    f.seed_old(&b);
    let other = DurableKernelStateStore::open_kernel_owned(&b).unwrap();
    assert!(!other.owns_skill_catalog());
    assert!(other.skills_v2_reset_complete().unwrap());
    assert_eq!(fs::read(artifact).unwrap(), before);
    let rows = other
        .load_subject_events_by_kind("session", "session.updated", 1)
        .unwrap();
    assert_eq!(
        rows.last().unwrap().payload["workflows"][0]["prompt"],
        "Use  then explain $total"
    );
    assert!(crate::skill::storage::SkillCatalogStorage::new(&f.home(), other.clone()).is_err());
    assert!(crate::skill::storage::SkillCatalogStorage::new(&f.home(), owner.clone()).is_ok());
}

#[test]
fn skills_v2_publication_startup_allows_an_unmaterialized_private_context_root() {
    let f = Fixture::new();
    let mut config = f.config("publication");
    config.publication_control_state_root = Some(f.0.join("publication-state"));
    let private = config.private_runtime_state_root();
    assert!(!private.exists());
    let store = DurableKernelStateStore::open_kernel_owned(&config).unwrap();
    assert!(store.skills_v2_reset_complete().unwrap());
    assert!(store.owns_skill_catalog());
    assert!(
        !private.exists(),
        "a missing read-side context root must not be reserved"
    );
}
