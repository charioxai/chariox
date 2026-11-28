//! MP-08 / MP-10 / MP-11: instruction tools use the ordinary runtime transport, without grants.
use super::RuntimeToolSpec;
use crate::skill::v2::SkillCatalogMutation;
use serde::Deserialize;
#[path = "skill_tool_schema.rs"]
mod schema;

pub const LIST_SKILLS_TOOL: &str = "chariox.list_skills";
pub const LOAD_SKILL_TOOL: &str = "chariox.load_skill";
pub const MANAGE_SKILLS_TOOL: &str = "chariox.manage_skills";

pub fn canonical_skill_tool_name(name: &str) -> Option<&'static str> {
    match name {
        LIST_SKILLS_TOOL
        | "list_skills"
        | "chariox_list_skills"
        | "mcp__chariox__list_skills"
        | "mcp__chariox__chariox_list_skills" => Some(LIST_SKILLS_TOOL),
        LOAD_SKILL_TOOL
        | "load_skill"
        | "chariox_load_skill"
        | "mcp__chariox__load_skill"
        | "mcp__chariox__chariox_load_skill" => Some(LOAD_SKILL_TOOL),
        MANAGE_SKILLS_TOOL
        | "manage_skills"
        | "chariox_manage_skills"
        | "mcp__chariox__manage_skills"
        | "mcp__chariox__chariox_manage_skills" => Some(MANAGE_SKILLS_TOOL),
        _ => None,
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSkillsArgs {
    pub origin_prompt_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadSkillArgs {
    pub skill_id: String,
    pub origin_prompt_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManageSkillsArgs {
    pub origin_prompt_id: String,
    pub request: ManageSkillsRequest,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManageSkillsRequest {
    Preview {
        mutation: SkillCatalogMutation,
    },
    Publish {
        mutation: SkillCatalogMutation,
        review_receipt_id: String,
    },
    ResolveRepository {
        url: String,
    },
    ResolveRepositoryUpdate {
        package_id: String,
    },
}
pub fn skill_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: LIST_SKILLS_TOOL.into(),
            description: "Discover Auto skills in this agent's Project and the home Chariox catalog. Returns stable IDs, this agent's project_id and a bounded page; follow next_cursor. Manual and Off skills are omitted. Use the current prompt_id as origin_prompt_id. Listing does not load instruction bodies or grant tools.".into(),
            input_schema: serde_json::json!({"type":"object","required":["origin_prompt_id"],"properties":{
                "origin_prompt_id":{"type":"string","minLength":1},
                "cursor":{"type":"string","maxLength":128},
                "limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: LOAD_SKILL_TOOL.into(),
            description: "Load pinned instructions and bundled-file locations for an Auto skill, or a Manual skill the user explicitly selected in the current kernel turn. Pass its listed name or its list_skills id as skill_id. Use the current prompt_id as origin_prompt_id. An explicit=true argument cannot confer authority. This never installs or grants referenced tools, credentials or hooks; ordinary sandbox/permission policy applies.".into(),
            input_schema: serde_json::json!({"type":"object","required":["skill_id","origin_prompt_id"],"properties":{
                "skill_id":{"type":"string","minLength":1},
                "origin_prompt_id":{"type":"string","minLength":1}},"additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: MANAGE_SKILLS_TOOL.into(),
            description: "Review and atomically publish a Skills batch through the home catalog. Use the current prompt_id as origin_prompt_id at the top level; put action and its fields inside request. First preview a mutation, then publish the identical mutation with its review_receipt_id. Mutation takes scope, expected_revision and actions. Scope {kind:chariox} applies across Projects (the default); {kind:project,project_id} limits it to this Project, using the ID from list_skills. Use that catalog's current revision; stale batches fail. The schema defines install/update/remove, state, package and move actions. Refresh a folder-backed skill with update, its skill_id/expected_digest and source {kind:recorded_folder}; preview reports changed files or no change. resolve_repository with url lists a public https repository's commit and skill paths without installing; preview one create_package plus installs pinned to that revision/path/package_id, then publish. resolve_repository_update with package_id returns changes and a ready mutation to preview and publish unchanged. Publication follows owner and ask/yolo policy, affects the next turn, keeps this turn pinned, and never executes bundled scripts or grants capabilities.".into(),
            input_schema: schema::manage(),
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    // MP-08 / MP-10 / MP-11: SV-14's real install omitted the turn binding.
    // The advertised example must be executable by the actual tool decoder.
    #[test]
    fn skill_manage_guidance_carries_current_prompt_identity() {
        let spec = skill_runtime_tool_specs()
            .into_iter()
            .find(|tool| tool.name == MANAGE_SKILLS_TOOL)
            .unwrap();
        assert!(spec
            .description
            .contains("current prompt_id as origin_prompt_id"));
        let origin_context = crate::prompt_assembly::agent_message_origin_context("prompt-1");
        assert!(origin_context.contains("runtime tool requiring origin_prompt_id"));
        assert!(origin_context.contains("preview and publish"));
        assert!(origin_context.contains("`prompt-1`"));
        let example = spec.input_schema["examples"][0].clone();
        assert!(serde_json::from_value::<ManageSkillsArgs>(example.clone()).is_ok());
        let schema = jsonschema::JSONSchema::compile(&spec.input_schema).unwrap();
        assert!(schema.is_valid(&example));
        let mut missing_origin = example;
        missing_origin
            .as_object_mut()
            .unwrap()
            .remove("origin_prompt_id");
        assert!(!schema.is_valid(&missing_origin));
        assert!(serde_json::from_value::<ManageSkillsArgs>(missing_origin).is_err());
    }
    #[test]
    fn skill_tools_do_not_accept_model_supplied_explicit_or_target_authority() {
        assert!(serde_json::from_value::<LoadSkillArgs>(serde_json::json!({
            "skill_id":"manual","origin_prompt_id":"turn","explicit":true}))
        .is_err());
        assert!(serde_json::from_value::<ListSkillsArgs>(
            serde_json::json!({"project_id":"other"})
        )
        .is_err());
        for tool in skill_runtime_tool_specs() {
            assert_eq!(
                canonical_skill_tool_name(&tool.name),
                Some(tool.name.as_str())
            );
        }
    }

    #[test]
    fn skill_manage_envelope_rejects_flat_and_ambiguous_requests() {
        let spec = skill_runtime_tool_specs()
            .into_iter()
            .find(|tool| tool.name == MANAGE_SKILLS_TOOL)
            .unwrap();
        let schema = jsonschema::JSONSchema::compile(&spec.input_schema).unwrap();
        for malformed in [
            serde_json::json!({"action":"resolve_repository","url":"https://github.com/org/skills","origin_prompt_id":"prompt-1"}),
            serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"resolve_repository","url":"https://github.com/org/skills"},"action":"resolve_repository"}),
            serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"resolve_repository","url":"https://github.com/org/skills","origin_prompt_id":"prompt-1"}}),
            serde_json::json!({"request":{"action":"resolve_repository","url":"https://github.com/org/skills"}}),
            serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"unknown"}}),
            serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"publish","mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[]}}}),
        ] {
            assert!(!schema.is_valid(&malformed), "admitted: {malformed}");
            assert!(serde_json::from_value::<ManageSkillsArgs>(malformed).is_err());
        }
        for invalid_request in [
            serde_json::json!({"action":"resolve_repository","url":""}),
            serde_json::json!({"action":"resolve_repository_update","package_id":"invalid/id"}),
            serde_json::json!({"action":"preview","mutation":{"scope":{"kind":"chariox"},"expected_revision":-1,"actions":[]}}),
        ] {
            assert!(!schema.is_valid(&serde_json::json!({
                "origin_prompt_id":"prompt-1","request":invalid_request,
            })));
        }
    }
    // MP-08 / MP-10 / MP-11: catch drift between advertised JSON schema and real Serde tags.
    #[test]
    fn skill_manage_schema_matches_canonical_batches_and_rejects_ambiguous_install_arguments() {
        use crate::skill::v2::*;
        let spec = skill_runtime_tool_specs()
            .into_iter()
            .find(|tool| tool.name == MANAGE_SKILLS_TOOL)
            .unwrap();
        let schema = jsonschema::JSONSchema::options()
            .with_draft(jsonschema::Draft::Draft7)
            .compile(&spec.input_schema)
            .unwrap();
        let source = SkillInstallSource::Folder {
            path: "skills/skill-creator".into(),
        };
        let scope = SkillScope::Chariox;
        let actions = vec![
            SkillCatalogAction::Install {
                source: source.clone(),
                collision: SkillCollisionAction::Reject,
                package_id: None,
            },
            SkillCatalogAction::Update {
                skill_id: "skill-1".into(),
                source: source.clone(),
                expected_digest: "a".repeat(64),
            },
            SkillCatalogAction::Update {
                skill_id: "skill-1".into(),
                source: SkillInstallSource::RecordedFolder {},
                expected_digest: "a".repeat(64),
            },
            SkillCatalogAction::Remove {
                skill_id: "skill-1".into(),
            },
            SkillCatalogAction::SetState {
                skill_id: "skill-1".into(),
                state: None,
            },
            SkillCatalogAction::CreatePackage {
                package_id: "package-1".into(),
                name: "Review package".into(),
            },
            SkillCatalogAction::RenamePackage {
                package_id: "package-1".into(),
                name: "Reviewed".into(),
            },
            SkillCatalogAction::Regroup {
                skill_id: "skill-1".into(),
                package_id: None,
            },
            SkillCatalogAction::RemovePackage {
                package_id: "package-1".into(),
                remove_skills: false,
            },
            SkillCatalogAction::SetPackageState {
                package_id: "package-1".into(),
                state: Some(SkillState::Manual),
            },
            SkillCatalogAction::Move {
                skill_id: "skill-1".into(),
                destination: SkillScope::Project {
                    project_id: "project-p".into(),
                },
            },
        ];
        for action in actions {
            let value = serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"preview","mutation":SkillCatalogMutation{scope:scope.clone(),expected_revision:0,actions:vec![action]}}});
            assert!(
                schema.is_valid(&value),
                "canonical mutation rejected: {value}"
            );
            assert!(serde_json::from_value::<ManageSkillsArgs>(value.clone()).is_ok());
            let mut publish = value.clone();
            publish["request"]["action"] = serde_json::json!("publish");
            assert!(!schema.is_valid(&publish));
            publish["request"]["review_receipt_id"] = serde_json::json!("review-1");
            assert!(schema.is_valid(&publish));
            assert!(serde_json::from_value::<ManageSkillsArgs>(publish).is_ok());
        }
        // SV-15: an agent imports and updates repository packages itself.
        let repository = serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"preview","mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[
            {"action":"create_package","package_id":"package-repo","name":"skills"},
            {"action":"install","source":{"kind":"repository","url":"https://github.com/org/skills","revision":"a".repeat(40),"path":""},"collision":{"action":"reject"},"package_id":"package-repo"}]}}});
        let resolve = serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"resolve_repository","url":"https://github.com/org/skills"}});
        let update = serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"resolve_repository_update","package_id":"package-repo"}});
        for value in [&repository, &resolve, &update] {
            assert!(schema.is_valid(value), "rejected: {value}");
            assert!(serde_json::from_value::<ManageSkillsArgs>(value.clone()).is_ok());
        }
        let mut resolve_with_mutation = resolve.clone();
        resolve_with_mutation["request"]["mutation"] = repository["request"]["mutation"].clone();
        let mut update_with_url = update.clone();
        update_with_url["request"]["url"] = resolve["request"]["url"].clone();
        let mut resolve_without_origin = resolve.clone();
        resolve_without_origin
            .as_object_mut()
            .unwrap()
            .remove("origin_prompt_id");
        let mut resolve_without_url = resolve.clone();
        resolve_without_url["request"]
            .as_object_mut()
            .unwrap()
            .remove("url");
        for malformed in [
            resolve_with_mutation,
            update_with_url,
            resolve_without_origin,
            resolve_without_url,
        ] {
            assert!(!schema.is_valid(&malformed), "admitted: {malformed}");
            assert!(serde_json::from_value::<ManageSkillsArgs>(malformed).is_err());
        }
        let base = serde_json::json!({"origin_prompt_id":"prompt-1","request":{"action":"preview","mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[{"action":"install","source":{"kind":"folder","path":"skills/skill-creator"},"collision":{"action":"reject"}}]}}});
        let mut wrong_tag = base.clone();
        let action = wrong_tag["request"]["mutation"]["actions"][0]
            .as_object_mut()
            .unwrap();
        action.remove("action");
        action.insert("kind".into(), serde_json::json!("install"));
        let mut wrong_source = base.clone();
        wrong_source["request"]["mutation"]["actions"][0]["source"] =
            serde_json::json!({"type":"folder","path":"skills/skill-creator"});
        let mut wrong_collision = base.clone();
        wrong_collision["request"]["mutation"]["actions"][0]["collision"] =
            serde_json::json!("reject");
        let mut wrong_receipt = base.clone();
        wrong_receipt["request"]["review_receipt_id"] = serde_json::json!("review-1");
        for malformed in [wrong_tag, wrong_source, wrong_collision, wrong_receipt] {
            assert!(serde_json::from_value::<ManageSkillsArgs>(malformed.clone()).is_err());
            assert!(
                !schema.is_valid(&malformed),
                "schema admits a request the real decoder rejects"
            );
        }
    }
}
