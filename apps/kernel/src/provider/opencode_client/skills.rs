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
impl OpenCodeClient {
    /// OpenCode lists the skills its instance loaded when it started. When the skill
    /// files changed since and no session is busy, dispose the instance through its
    /// documented `instance.dispose` so the next request loads them again; sessions
    /// stay on disk, so the conversation continues. Returns whether it reloaded.
    pub fn reload_stale_skills(
        &self,
        cwd: &std::path::Path,
        listed: &[NativeSkillCandidate],
        on_disk: &[NativeSkillCandidate],
    ) -> Result<bool, DaemonError> {
        if !opencode_listing_is_stale(listed, on_disk) {
            return Ok(false);
        }
        let schema: Value = self.send_json_request("GET", "/doc", None)?;
        if !schema_advertises_method(&schema, "/instance/dispose", "post", "instance.dispose")
            || !schema_advertises(&schema, "/session/status", "session.status")
        {
            return Ok(false);
        }
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("directory", &cwd.to_string_lossy())
            .finish();
        let status: Value =
            self.send_json_request("GET", &format!("/session/status?{query}"), None)?;
        let busy = status.as_object().is_none_or(|sessions| {
            sessions
                .values()
                .any(|session| session.get("type").and_then(Value::as_str) != Some("idle"))
        });
        if busy {
            return Ok(false);
        }
        let _: Value =
            self.send_json_request("POST", &format!("/instance/dispose?{query}"), None)?;
        Ok(true)
    }
}

/// A skill-only root holds a skill OpenCode does not list, or a listed skill file is
/// gone or holds another body than the one OpenCode retained.
fn opencode_listing_is_stale(
    listed: &[NativeSkillCandidate],
    on_disk: &[NativeSkillCandidate],
) -> bool {
    use sha2::{Digest, Sha256};
    let names: std::collections::BTreeSet<&str> =
        listed.iter().map(|skill| skill.name.as_str()).collect();
    on_disk
        .iter()
        .any(|skill| !names.contains(skill.name.as_str()))
        || listed
            .iter()
            .filter(|skill| skill.skill_file.is_absolute())
            .any(|skill| {
                let Ok(text) = std::fs::read_to_string(&skill.skill_file) else {
                    return true;
                };
                let body = text
                    .strip_prefix("---\n")
                    .and_then(|rest| rest.split_once("\n---\n"))
                    .map_or(text.as_str(), |(_, body)| body);
                skill
                    .observed_body_digest
                    .as_deref()
                    .is_some_and(|retained| {
                        retained != format!("{:x}", Sha256::digest(body.trim().as_bytes()))
                    })
            })
}

fn schema_advertises(schema: &Value, path: &str, operation: &str) -> bool {
    schema_advertises_method(schema, path, "get", operation)
}
fn schema_advertises_method(schema: &Value, path: &str, method: &str, operation: &str) -> bool {
    schema
        .get("paths")
        .and_then(|v| v.get(path))
        .and_then(|v| v.get(method))
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
                plugin: None,
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
    fn a_listing_is_stale_when_skill_files_were_added_changed_or_removed() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("opencode-stale-{}", std::process::id()));
        let file = |name: &str| root.join(name).join("SKILL.md");
        let write = |name: &str, body: &str| {
            std::fs::create_dir_all(file(name).parent().unwrap()).unwrap();
            std::fs::write(
                file(name),
                format!("---\nname: {name}\ndescription: d\n---\n{body}"),
            )
            .unwrap();
        };
        let candidate = |name: &str, retained: Option<&str>| NativeSkillCandidate {
            name: name.into(),
            description: "d".into(),
            skill_file: file(name),
            exposure: NativeSkillExposure::Confirmed,
            supports_load_ack: true,
            observed_body_digest: retained
                .map(|body| format!("{:x}", Sha256::digest(body.trim().as_bytes()))),
            discovery_error: None,
            plugin: None,
        };
        write("one", "One.\n");
        let listed = vec![candidate("one", Some("One.\n"))];
        assert!(!opencode_listing_is_stale(
            &listed,
            &[candidate("one", None)]
        ));
        write("two", "Two.\n");
        assert!(opencode_listing_is_stale(
            &listed,
            &[candidate("one", None), candidate("two", None)]
        ));
        write("one", "Changed.\n");
        assert!(opencode_listing_is_stale(
            &listed,
            &[candidate("one", None)]
        ));
        std::fs::remove_dir_all(root.join("one")).unwrap();
        assert!(opencode_listing_is_stale(&listed, &[]));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn permission_globs_do_not_interpret_regex() {
        assert!(glob_matches("internal-*", "internal-docs"));
        assert!(glob_matches("r?view", "review"));
        assert!(!glob_matches("review.*", "review-other"));
    }
}
