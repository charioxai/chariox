//! MP-08 / MP-11: parse YAML as data. Provider metadata cannot execute or grant.
use super::{
    catalog::{collision_key, failure},
    v2::SkillState,
};
use crate::error::DaemonError;
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub declared_version: Option<String>,
    pub default_state: SkillState,
    pub compatibility_notes: Vec<String>,
}

pub fn parse_frontmatter(body: &str) -> Result<SkillFrontmatter, DaemonError> {
    let mut lines = body.lines();
    if lines.next() != Some("---") {
        return Err(failure("SKILL.md must start with YAML frontmatter"));
    }
    let mut yaml = String::new();
    let mut terminated = false;
    for line in lines {
        if line == "---" {
            terminated = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !terminated {
        return Err(failure("SKILL.md frontmatter is not terminated"));
    }
    let value: Value =
        serde_yaml::from_str(&yaml).map_err(|_| failure("invalid SKILL.md YAML frontmatter"))?;
    let map = value
        .as_mapping()
        .ok_or_else(|| failure("skill frontmatter must be a YAML mapping"))?;
    let name = text(map, "name")?.ok_or_else(|| failure("skill frontmatter requires a name"))?;
    collision_key(&name)?;
    let description = text(map, "description")?
        .ok_or_else(|| failure("skill frontmatter requires a description"))?;
    if description.trim().is_empty() {
        return Err(failure("skill description must not be empty"));
    }
    if map.contains_key(Value::String("disable_model_invocation".into())) {
        return Err(failure(
            "use the YAML boolean disable-model-invocation field",
        ));
    }
    if let Some(value) = map.get(Value::String("user-invocable".into())) {
        if !value.is_bool() {
            return Err(failure(
                "user-invocable must be a YAML boolean when present",
            ));
        }
    }
    let disabled = match map.get(Value::String("disable-model-invocation".into())) {
        None => false,
        Some(Value::Bool(v)) => *v,
        _ => {
            return Err(failure(
                "disable-model-invocation must be a YAML boolean (true or false)",
            ))
        }
    };
    let nested_short = map
        .get(Value::String("metadata".into()))
        .and_then(Value::as_mapping)
        .map(|metadata| text(metadata, "short-description"))
        .transpose()?
        .flatten();
    let short_description = text(map, "short-description")?
        .or(text(map, "short_description")?)
        .or(nested_short);
    let declared_version = text(map, "version")?;
    let mut compatibility_notes = Vec::new();
    for key in [
        "allowed-tools",
        "hooks",
        "context",
        "agent",
        "fork",
        "user-invocable",
    ] {
        if map.contains_key(Value::String(key.into())) {
            compatibility_notes.push(format!(
                "{key} is retained in source only; Chariox does not execute it or grant authority"
            ));
        }
    }
    if body.contains("!`") {
        compatibility_notes.push("Dynamic shell substitutions are instructions only and are never executed by skill loading".into());
    }
    Ok(SkillFrontmatter {
        name,
        description,
        short_description,
        declared_version,
        default_state: if disabled {
            SkillState::Manual
        } else {
            SkillState::Auto
        },
        compatibility_notes,
    })
}

fn text(map: &Mapping, key: &str) -> Result<Option<String>, DaemonError> {
    match map.get(Value::String(key.into())) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(failure(format!(
            "skill frontmatter `{key}` must be a string"
        ))),
    }
}
