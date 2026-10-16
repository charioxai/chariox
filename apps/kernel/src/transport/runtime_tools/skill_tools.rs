//! MP-08 / MP-10 / MP-11: instruction tools use the ordinary runtime transport, without grants.
use super::RuntimeToolSpec;
use crate::skill::v2::SkillCatalogMutation;
use serde::Deserialize;

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
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManageSkillsArgs {
    Preview {
        mutation: SkillCatalogMutation,
        origin_prompt_id: String,
    },
    Publish {
        mutation: SkillCatalogMutation,
        review_receipt_id: String,
        origin_prompt_id: String,
    },
}
pub fn skill_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: LIST_SKILLS_TOOL.into(),
            description: "Discover Auto skills in this agent's Project and the home Chariox catalog. Returns stable IDs and a bounded page; follow next_cursor. Manual and Off skills are omitted. Use the current prompt_id as origin_prompt_id. Listing does not load instruction bodies or grant tools.".into(),
            input_schema: serde_json::json!({"type":"object","required":["origin_prompt_id"],"properties":{
                "origin_prompt_id":{"type":"string","minLength":1},
                "cursor":{"type":"string","maxLength":128},
                "limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: LOAD_SKILL_TOOL.into(),
            description: "Load pinned instructions and bundled-file locations for an Auto skill, or a Manual skill the user explicitly selected in the current kernel turn. Use the current prompt_id as origin_prompt_id. An explicit=true argument cannot confer authority. This never installs or grants referenced tools, credentials or hooks; ordinary sandbox/permission policy applies.".into(),
            input_schema: serde_json::json!({"type":"object","required":["skill_id","origin_prompt_id"],"properties":{
                "skill_id":{"type":"string","minLength":1},
                "origin_prompt_id":{"type":"string","minLength":1}},"additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: MANAGE_SKILLS_TOOL.into(),
            description: "Review and atomically publish a Skills catalog batch through the home catalog. First preview, then publish the identical mutation with its review_receipt_id. Mutation has scope ({kind:chariox} or {kind:project,project_id}), expected_revision, and actions. Actions use the Skills v2 install/update/remove/state/package contract. Install source is {kind:folder,path}, {kind:content,skill_md} or {kind:url,url}; collision is {action:reject}, {action:rename,name} or {action:replace,skill_id}. Publication follows normal owner and ask/yolo policy. It affects the next turn, keeps this turn pinned, and never executes bundled scripts or grants capabilities.".into(),
            input_schema: serde_json::json!({"type":"object","required":["action","mutation","origin_prompt_id"],"properties":{
                "action":{"type":"string","enum":["preview","publish"]},
                "mutation":{"type":"object","required":["scope","expected_revision","actions"],"properties":{
                    "scope":{"type":"object"},"expected_revision":{"type":"integer","minimum":0},
                    "actions":{"type":"array","minItems":1,"maxItems":256,"items":{"type":"object"}}},"additionalProperties":false},
                "review_receipt_id":{"type":"string","minLength":1},
                "origin_prompt_id":{"type":"string","minLength":1}},"additionalProperties":false}),
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
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
}
