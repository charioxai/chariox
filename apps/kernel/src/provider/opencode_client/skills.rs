//! MP-08 / MP-10 / MP-11: inspect the pinned official schema before inventory.
use super::OpenCodeClient;
use crate::{
    error::DaemonError, provider::skill_inventory::NativeSkillCandidate,
    skill::v2::NativeSkillExposure,
};
use serde_json::Value;

impl OpenCodeClient {
    /// The 1.19.0 schema advertises app.skills plus app.agents. No assumed
    /// stable listing endpoint: an unsupported schema returns None so callers
    /// can expose skill-only filesystem candidates explicitly as unverified.
    pub fn native_skill_candidates(
        &self,
        cwd: &std::path::Path,
        agent: &str,
    ) -> Result<Option<Vec<NativeSkillCandidate>>, DaemonError> {
        let schema: Value = self.send_json_request("GET", "/doc", None)?;
        if !schema_advertises(&schema, "/skill", "app.skills")
            || !schema_advertises(&schema, "/agent", "app.agents")
        {
            return Ok(None);
        }
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("directory", &cwd.to_string_lossy())
            .finish();
        let skills: Value = self.send_json_request("GET", &format!("/skill?{query}"), None)?;
        let agents: Value = self.send_json_request("GET", &format!("/agent?{query}"), None)?;
        opencode_skill_candidates(&skills, &agents, agent).map(Some)
    }
}
fn schema_advertises(schema: &Value, path: &str, operation: &str) -> bool {
    schema
        .get("paths")
        .and_then(|v| v.get(path))
        .and_then(|v| v.get("get"))
        .and_then(|v| v.get("operationId"))
        .and_then(Value::as_str)
        == Some(operation)
}
fn opencode_skill_candidates(
    skills: &Value,
    agents: &Value,
    agent: &str,
) -> Result<Vec<NativeSkillCandidate>, DaemonError> {
    let failure = || DaemonError::LocalTransport {
        operation: "opencode.skill_inventory",
        message: "official skill/agent inventory omitted effective exposure data".into(),
    };
    let skills = skills.as_array().ok_or_else(failure)?;
    let agent = agents
        .as_array()
        .and_then(|agents| {
            agents
                .iter()
                .find(|v| v.get("name").and_then(Value::as_str) == Some(agent))
        })
        .ok_or_else(failure)?;
    let permissions = agent
        .get("permission")
        .and_then(Value::as_array)
        .ok_or_else(failure)?;
    Ok(skills
        .iter()
        .filter_map(|skill| {
            let name = skill.get("name")?.as_str()?;
            let mut exposure = NativeSkillExposure::Confirmed;
            for rule in permissions {
                // OpenCode evaluates normalized rules in order, last match wins.
                let permission = rule.get("permission").and_then(Value::as_str).unwrap_or("");
                if permission != "skill" && permission != "*" {
                    continue;
                }
                let Some(pattern) = rule.get("pattern").and_then(Value::as_str) else {
                    exposure = NativeSkillExposure::Unverified;
                    continue;
                };
                if !glob_matches(pattern, name) {
                    continue;
                }
                exposure = match rule.get("action").and_then(Value::as_str) {
                    Some("deny") => NativeSkillExposure::Disabled,
                    // Ask remains exposed; the official skill tool must retain its native approval.
                    Some("ask") => NativeSkillExposure::Confirmed,
                    Some("allow") => NativeSkillExposure::Confirmed,
                    _ => NativeSkillExposure::Unverified,
                };
            }
            let observed_body = skill.get("content").and_then(Value::as_str);
            if observed_body.is_none() && exposure != NativeSkillExposure::Disabled {
                exposure = NativeSkillExposure::Unverified;
            }
            Some(NativeSkillCandidate {
                name: name.into(),
                description: skill
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                skill_file: skill
                    .get("location")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                exposure,
                supports_load_ack: true,
                observed_body_digest: observed_body.map(|body| {
                    use sha2::{Digest, Sha256};
                    format!("{:x}", Sha256::digest(body.trim().as_bytes()))
                }),
                discovery_error: observed_body
                    .is_none()
                    .then(|| "official native inventory did not provide its retained body".into()),
            })
        })
        .collect())
}
// OpenCode's permission patterns support * and ?. Operate on Unicode scalars;
// no regex interpretation and no filesystem glob expansion.
fn glob_matches(pattern: &str, name: &str) -> bool {
    let chars: Vec<_> = name.chars().collect();
    let mut previous = vec![false; chars.len() + 1];
    previous[0] = true;
    for p in pattern.chars() {
        let mut next = vec![false; chars.len() + 1];
        if p == '*' {
            next[0] = previous[0];
        }
        for i in 1..=chars.len() {
            next[i] = if p == '*' {
                previous[i] || next[i - 1]
            } else {
                previous[i - 1] && (p == '?' || p == chars[i - 1])
            };
        }
        previous = next;
    }
    previous[chars.len()]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_advertised_schema_not_a_guessed_endpoint() {
        assert!(!schema_advertises(
            &serde_json::json!({"paths":{"/skill":{"get":{"operationId":"other"}}}}),
            "/skill",
            "app.skills"
        ));
        assert!(schema_advertises(
            &serde_json::json!({"paths":{"/skill":{"get":{"operationId":"app.skills"}}}}),
            "/skill",
            "app.skills"
        ));
    }
    #[test]
    fn effective_agent_rules_distinguish_denied_ask_and_allowed() {
        let skills = serde_json::json!([
            {"name":"private-review","description":"Review","content":"body","location":"/skills/private/SKILL.md"},
            {"name":"review","description":"Review","content":"body","location":"/skills/review/SKILL.md"},
            {"name":"experimental","description":"Review","content":"body","location":"/skills/exp/SKILL.md"}]);
        let agents = serde_json::json!([{"name":"build","permission":[
            {"permission":"*","pattern":"*","action":"allow"},
            {"permission":"skill","pattern":"private-*","action":"deny"},
            {"permission":"skill","pattern":"experimental","action":"ask"}]}]);
        let entries = opencode_skill_candidates(&skills, &agents, "build").unwrap();
        assert_eq!(entries[0].exposure, NativeSkillExposure::Disabled);
        assert_eq!(entries[1].exposure, NativeSkillExposure::Confirmed);
        assert_eq!(entries[2].exposure, NativeSkillExposure::Confirmed);
        assert!(opencode_skill_candidates(&skills, &agents, "absent").is_err());
    }
    #[test]
    fn permission_globs_do_not_interpret_regex() {
        assert!(glob_matches("internal-*", "internal-docs"));
        assert!(glob_matches("r?view", "review"));
        assert!(!glob_matches("review.*", "review-other"));
    }
}
