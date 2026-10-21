// MP-08 / MP-10 / MP-11: a `$name` typed in Claude's own composer on a leased worker
// resolves against the home's name view and the lease's verified cache.
use super::skill_hook_fakes::{DeliverSkill, PassthroughPolicy, TYPED_RECEIPTS};
use super::*;
use crate::skill::{
    leased_names::{LeasedSkillName, LeasedSkillNames},
    v2::{SkillScope, SkillState, SkillTransferEntry, SkillTransferManifest},
};

const BODY_MARKER: &str = "WORKER-TYPED-BODY-MARKER-31";

struct Worker {
    app: DaemonApp,
    backing_session_id: String,
    backing_agent_id: String,
    skill_id: String,
    digest: String,
    root: std::path::PathBuf,
    scratch: std::path::PathBuf,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

impl Worker {
    /// A worker kernel whose lease holds one skill named `review`.
    fn with_review_skill(state: SkillState) -> Self {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let root = std::env::temp_dir().join(format!(
            "chariox-typed-leased-{}-{}",
            std::process::id(),
            timestamp_millis()
        ));
        let worktree = root.join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        let source = root.join("review");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: review\ndescription: Review code\n---\n{BODY_MARKER}\n"),
        )
        .unwrap();
        let package = crate::skill::package_skill_directory(&source).unwrap();
        let mut runtime = crate::app::RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                &format!("home-session-{}", timestamp_millis()),
                "home-agent",
                false,
                "local-user",
            )
            .unwrap();
        let leased = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                "default",
                None,
                None,
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .unwrap();
        let skill_id = "skill-review".to_string();
        let manifest = SkillTransferManifest {
            catalog_revision: 1,
            project_id: None,
            skills: vec![SkillTransferEntry {
                skill_id: skill_id.clone(),
                digest: package.version_hash.clone(),
                scope: SkillScope::Chariox,
                state,
            }],
        };
        let names = LeasedSkillNames {
            catalog_revision: 1,
            skills: vec![LeasedSkillName {
                skill_id: skill_id.clone(),
                name: "review".into(),
                digest: package.version_hash.clone(),
            }],
        };
        let materialized = runtime
            .ensure_remote_skill_packages(
                crate::transport::relay_peer::RemoteSkillSyncContext {
                    home_kernel_id: lease.home_kernel_id.clone(),
                    home_session_id: lease.home_session_id.clone(),
                    home_agent_id: lease.home_agent_id.clone(),
                    leased_agent_id: leased.id.clone(),
                },
                lease.id.clone(),
                manifest,
                vec![package.clone()],
                Some(names),
            )
            .unwrap();
        Self {
            app,
            backing_session_id: leased.backing_session_id,
            backing_agent_id: leased.backing_agent_id,
            skill_id,
            digest: package.version_hash,
            root: std::path::PathBuf::from(&materialized[0].materialized_root),
            scratch: root,
        }
    }

    /// Types `prompt` into the worker's Claude composer and returns the hook context.
    fn type_prompt(&mut self, run_id: &str, prompt: &str) -> (String, String) {
        self.app
            .providers
            .set_skill_prompt_bridge(std::sync::Arc::new(DeliverSkill));
        let files = self.scratch.join(run_id);
        fs::create_dir_all(&files).unwrap();
        let context_file = files.join("hidden-context.txt");
        let events_file = files.join("events.jsonl");
        fs::write(&context_file, "").unwrap();
        fs::write(&events_file, "").unwrap();
        let mut run = startup_readiness_run(
            &self.backing_session_id,
            &self.backing_agent_id,
            run_id,
            &context_file,
            &events_file,
            "cat >/dev/null".to_string(),
        );
        run.mark_running();
        let mut ready_run = serde_json::to_value(&run).unwrap();
        ready_run["started_at_ms"] = serde_json::json!(unix_epoch_ms().saturating_sub(5_000));
        ready_run["pty_env"]["CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS"] = serde_json::json!("1");
        let run: RuntimeProviderRun = serde_json::from_value(ready_run).unwrap();
        self.app.pty.spawn_for_run(&run).unwrap();
        self.app.providers_mut().insert_run_for_test(run.clone());
        self.app
            .sessions
            .set_active_provider_run(&self.backing_session_id, Some(run.id().to_string()))
            .unwrap();
        let request_id = format!("{run_id}-request");
        let policy = PassthroughPolicy;
        let mut send = |event: serde_json::Value| {
            fs::write(&events_file, event.to_string()).unwrap();
            ProviderOutputClaudeNativeBridge::new(&mut self.app)
                .with_prompt_handoff(&policy)
                .process(&self.backing_session_id, run.id(), &run, None)
                .unwrap();
        };
        send(serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": prompt,
            "hook_context_request_id": request_id,
        }));
        let delivered =
            fs::read_to_string(files.join(format!("hook-context-responses/{request_id}.txt")))
                .unwrap();
        send(serde_json::json!({
            "hook_event_name": "CharioxContextDelivered",
            "hook_context_request_id": request_id,
        }));
        let prompt_id = self
            .app
            .prompt_owner_active_prompt_for_agent(&self.backing_session_id, &self.backing_agent_id)
            .unwrap()
            .unwrap()
            .id()
            .to_string();
        self.app.pty.remove_process(run.id()).unwrap();
        (delivered, prompt_id)
    }

    fn receipts_for(&self, prompt_id: &str) -> Vec<(String, String, String, Option<String>)> {
        TYPED_RECEIPTS
            .lock()
            .unwrap()
            .iter()
            .filter(|receipt| receipt.0 == prompt_id)
            .cloned()
            .collect()
    }
}

#[test]
fn leased_worker_typed_skill_reaches_the_hook_context_and_reports_its_load_home() {
    let mut worker = Worker::with_review_skill(SkillState::Manual);
    let (delivered, prompt_id) = worker.type_prompt("typed-leased-run", "$review inspect source");
    assert!(
        delivered.contains(BODY_MARKER),
        "the worker never delivered the typed skill: {delivered:?}"
    );
    assert!(
        delivered.contains(&format!(
            "Skill review ({}) at {}",
            worker.digest,
            worker.root.display()
        )),
        "the delivery lost the verified worker root: {delivered:?}"
    );
    assert_eq!(
        worker.receipts_for(&prompt_id),
        vec![(
            prompt_id.clone(),
            worker.skill_id.clone(),
            worker.digest.clone(),
            // The home has no dispatched turn for text typed on the worker.
            None
        )]
    );
}

#[test]
fn leased_worker_dollar_text_the_home_view_does_not_name_is_not_an_invocation() {
    let mut worker = Worker::with_review_skill(SkillState::Auto);
    let (delivered, prompt_id) = worker.type_prompt(
        "untyped-leased-run",
        "pay $5 for $unknown and $HOME, not `$review`",
    );
    assert!(
        !delivered.contains(BODY_MARKER),
        "dollar text became an invocation: {delivered:?}"
    );
    assert!(worker.receipts_for(&prompt_id).is_empty());
}
