use crate::error::DaemonError;
use crate::transport::relay_peer::{
    RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteSkill,
};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn required_remote_skill_prompt_context(
        &self,
        leased_agent: &crate::execution_lease::LeasedAgent,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let backing_session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        crate::skill::format_granted_skill_prompt_context(
            backing_agent.agent_ref(),
            &backing_agent.skill_grants(),
            backing_session.workspace_id(),
            prompt,
        )
    }

    pub(super) fn apply_required_remote_skills(
        &mut self,
        leased_agent: &crate::execution_lease::LeasedAgent,
        required_skills: &[RequiredRemoteSkill],
    ) -> Result<(), DaemonError> {
        let _ = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let registry = crate::skill::CharioxSkillRegistry::new(
            crate::skill::CharioxSkillRegistry::user_root()
                .map(|root| vec![root])
                .unwrap_or_default(),
        );
        for required in required_skills {
            let package =
                registry
                    .package(&required.name)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "remote skill availability",
                        message: format!(
                            "remote agent `{}` requires skill `{}` which is missing on worker",
                            leased_agent.id, required.name
                        ),
                    })?;
            if package.version_hash != required.version_hash {
                return Err(DaemonError::LocalTransport {
                    operation: "remote skill availability",
                    message: format!(
                        "remote agent `{}` requires skill `{}` hash {}, but worker has {}",
                        leased_agent.id, required.name, required.version_hash, package.version_hash
                    ),
                });
            }
        }

        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let required_names = required_skills
            .iter()
            .map(|required| required.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for existing in backing_agent.skill_grants() {
            if !required_names.contains(existing.as_str()) {
                self.app
                    .agents
                    .revoke_skill(&leased_agent.backing_agent_id, &existing)?;
            }
        }
        for required in required_skills {
            if !backing_agent.skill_grants().contains(&required.name) {
                self.app
                    .agents
                    .grant_skill(&leased_agent.backing_agent_id, required.name.clone())?;
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_remote_skill_packages(
        &mut self,
        context: RemoteSkillSyncContext,
        packages: Vec<crate::skill::CharioxSkillPackage>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message: "remote skill sync context does not match leased agent".to_string(),
            });
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let base_dir = crate::skill::remote_skill_materialization_base(session.worktree_id())
            .join(&context.home_kernel_id);
        let install_into_isolated_registry = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
            .is_some_and(|value| !value.is_empty());
        let registry = install_into_isolated_registry.then(|| {
            crate::skill::CharioxSkillRegistry::new(
                crate::skill::CharioxSkillRegistry::user_root()
                    .map(|root| vec![root])
                    .unwrap_or_default(),
            )
        });
        packages
            .iter()
            .map(|package| {
                let materialized_root =
                    crate::skill::materialize_skill_package(&base_dir, package)?;
                if let Some(registry) = registry.as_ref() {
                    if registry.get(&package.metadata.name)?.is_none() {
                        registry.install_from_path(&materialized_root)?;
                    }
                }
                Ok(RemoteSkillMaterialization {
                    name: package.metadata.name.clone(),
                    version_hash: package.version_hash.clone(),
                    materialized_root: materialized_root.to_string_lossy().to_string(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DaemonApp;
    use crate::config::DaemonConfig;

    fn create_leased_agent(
        app: &mut DaemonApp,
        worktree: &std::path::Path,
    ) -> crate::execution_lease::LeasedAgent {
        let mut runtime = RemoteLeaseRuntime::new(app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "local-user",
            )
            .expect("lease should create");
        runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .expect("leased agent should create")
    }

    #[test]
    fn required_remote_skills_validate_hash_and_replace_backing_grants() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-required-remote-skill-test-{}",
            std::process::id()
        ));
        let worktree = root.join("worktree");
        let source = root.join("source").join("review");
        let isolation_root = root.join("capabilities");
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        std::fs::create_dir_all(&source).expect("skill source should create");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: review\ndescription: Review code.\n---\nFollow the review contract.\n",
        )
        .expect("skill source should write");
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        let registry = crate::skill::CharioxSkillRegistry::new(vec![
            crate::skill::CharioxSkillRegistry::user_root()
                .expect("isolated skill root should resolve"),
        ]);
        registry
            .install_from_path(&source)
            .expect("worker skill should install");
        let package = registry
            .package("review")
            .expect("skill package should load")
            .expect("skill package should exist");

        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let leased_agent = create_leased_agent(&mut app, &worktree);
        let mismatch = RemoteLeaseRuntime::new(&mut app).apply_required_remote_skills(
            &leased_agent,
            &[RequiredRemoteSkill {
                name: "review".to_string(),
                version_hash: "wrong-hash".to_string(),
            }],
        );
        assert!(mismatch
            .expect_err("mismatched worker skill should fail")
            .to_string()
            .contains("but worker has"));
        assert!(app
            .agents()
            .get_agent(&leased_agent.backing_agent_id)
            .expect("backing agent should load")
            .skill_grants()
            .is_empty());

        RemoteLeaseRuntime::new(&mut app)
            .apply_required_remote_skills(
                &leased_agent,
                &[RequiredRemoteSkill {
                    name: "review".to_string(),
                    version_hash: package.version_hash,
                }],
            )
            .expect("matching worker skill should apply");
        assert_eq!(
            app.agents()
                .get_agent(&leased_agent.backing_agent_id)
                .expect("backing agent should load")
                .skill_grants(),
            vec!["review".to_string()]
        );

        RemoteLeaseRuntime::new(&mut app)
            .apply_required_remote_skills(&leased_agent, &[])
            .expect("empty exact set should revoke worker skill grants");
        assert!(app
            .agents()
            .get_agent(&leased_agent.backing_agent_id)
            .expect("backing agent should load")
            .skill_grants()
            .is_empty());

        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn required_remote_skills_reject_missing_worker_package() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-missing-required-remote-skill-test-{}",
            std::process::id()
        ));
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        std::env::set_var(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            root.join("capabilities"),
        );
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let leased_agent = create_leased_agent(&mut app, &worktree);

        let result = RemoteLeaseRuntime::new(&mut app).apply_required_remote_skills(
            &leased_agent,
            &[RequiredRemoteSkill {
                name: "missing".to_string(),
                version_hash: "missing-hash".to_string(),
            }],
        );

        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(root);
        assert!(result
            .expect_err("missing worker skill should fail")
            .to_string()
            .contains("missing on worker"));
    }

    #[test]
    fn leased_prompt_dispatch_includes_worker_local_required_skill_context() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-required-remote-skill-prompt-test-{}",
            std::process::id()
        ));
        let worktree = root.join("worktree");
        let source = root.join("source").join("review");
        let isolation_root = root.join("capabilities");
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        std::fs::create_dir_all(&source).expect("skill source should create");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: review\ndescription: Review code.\n---\nReply with exactly WORKER_LOCAL_SKILL_CONTEXT.\n",
        )
        .expect("skill source should write");
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        let registry = crate::skill::CharioxSkillRegistry::new(vec![
            crate::skill::CharioxSkillRegistry::user_root()
                .expect("isolated skill root should resolve"),
        ]);
        registry
            .install_from_path(&source)
            .expect("worker skill should install");
        let package = registry
            .package("review")
            .expect("skill package should load")
            .expect("skill package should exist");

        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let leased_agent = create_leased_agent(&mut app, &worktree);
        let result = RemoteLeaseRuntime::new(&mut app).submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "Use the review skill.",
            Vec::new(),
            None,
            None,
            Vec::new(),
            Some(vec![RequiredRemoteSkill {
                name: "review".to_string(),
                version_hash: package.version_hash,
            }]),
            crate::extension::RemoteExtensionManifest::default(),
        );
        let provider_input = app
            .terminal()
            .input_records()
            .last()
            .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
            .unwrap_or_default();

        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(root);
        result.expect("leased prompt should submit");
        assert!(
            provider_input.contains("WORKER_LOCAL_SKILL_CONTEXT"),
            "worker-local skill instructions must reach provider input: {provider_input:?}"
        );
    }
}
