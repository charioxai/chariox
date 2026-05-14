//! Granted-skill prompt context injection.
//!
//! This module owns adding granted Arroba skill summaries and explicitly requested skill bodies to
//! prompts before provider submission.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn apply_granted_skill_summary(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.skill_grants().is_empty() {
            return Ok(prompt.to_string());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace = std::path::PathBuf::from(session.workspace_id());
        let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
        if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::skill::ArrobaSkillRegistry::new(roots);
        let mut lines = vec![
            "Available Arroba skills for this agent:".to_string(),
            "Use these granted skills as routing hints when they match the task. If a skill is explicitly selected, mentioned, or requested below, follow its full instructions.".to_string(),
        ];
        let mut requested_skill_bodies = Vec::new();
        for grant in agent.skill_grants() {
            let Some(skill) = registry.get(grant)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "provider.prompt.skills",
                    message: format!(
                        "agent `{}` has missing skill grant `{grant}`",
                        agent.agent_ref()
                    ),
                });
            };
            let summary = skill
                .short_description
                .as_ref()
                .unwrap_or(&skill.description);
            lines.push(format!("- `{}`: {}", skill.name, summary));
            if prompt_explicitly_requests_skill(prompt, &skill.name) {
                let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "provider.prompt.skills",
                        message: format!(
                            "failed to read skill `{}` body at `{}`: {error}",
                            skill.name,
                            skill.path.display()
                        ),
                    }
                })?;
                requested_skill_bodies.push((skill.name, body));
            }
        }
        if !requested_skill_bodies.is_empty() {
            lines.push(String::new());
            lines.push("Full instructions for explicitly requested Arroba skills:".to_string());
            for (name, body) in requested_skill_bodies {
                lines.push(format!("<arroba_skill name=\"{name}\">"));
                lines.push(body.trim().to_string());
                lines.push("</arroba_skill>".to_string());
            }
        }
        Ok(format!("{}\n\n{}", lines.join("\n"), prompt))
    }
}

fn prompt_explicitly_requests_skill(prompt: &str, skill_name: &str) -> bool {
    let prompt = prompt.to_lowercase();
    let skill_name = skill_name.to_lowercase();
    let explicit_markers = [
        format!("@{skill_name}"),
        format!("`{skill_name}`"),
        format!("/skill {skill_name}"),
        format!("skill {skill_name}"),
        format!("use {skill_name}"),
        format!("using {skill_name}"),
        format!("with {skill_name}"),
    ];
    explicit_markers
        .iter()
        .any(|marker| prompt.contains(marker))
        || contains_tokenish_skill_name(&prompt, &skill_name)
}

fn contains_tokenish_skill_name(prompt: &str, skill_name: &str) -> bool {
    prompt.match_indices(skill_name).any(|(index, _)| {
        let before = index
            .checked_sub(1)
            .and_then(|before| prompt.as_bytes().get(before))
            .copied();
        let after = prompt.as_bytes().get(index + skill_name.len()).copied();
        is_skill_boundary(before) && is_skill_boundary(after)
    })
}

fn is_skill_boundary(byte: Option<u8>) -> bool {
    byte.is_none_or(|byte| !byte.is_ascii_alphanumeric() && byte != b'-' && byte != b'_')
}

#[cfg(test)]
mod tests {
    use super::prompt_explicitly_requests_skill;

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
}
