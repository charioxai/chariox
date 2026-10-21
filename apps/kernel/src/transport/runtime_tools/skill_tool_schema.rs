//! MP-08 / MP-10 / MP-11: model mutation schemas use the actual tagged catalog contract.
use serde_json::{json, Value};
fn object(required: &[&str], properties: Value) -> Value {
    json!({"type":"object","required":required,"properties":properties,"additionalProperties":false})
}
fn variant(tag: &str, name: &str, required: &[&str], properties: Value) -> Value {
    let mut properties = properties.as_object().unwrap().clone();
    properties.insert(tag.into(), json!({"type":"string","enum":[name]}));
    let mut required = required.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    required.push(tag.into());
    json!({"type":"object","required":required,"properties":properties,"additionalProperties":false})
}
pub(super) fn manage() -> Value {
    let id = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"});
    let text = json!({"type":"string","minLength":1});
    let name = json!({"type":"string","minLength":1,"maxLength":128});
    let optional_id = json!({"type":["string","null"],"minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"});
    let scope = json!({"oneOf":[variant("kind","chariox",&[],json!({})),
        variant("kind","project",&["project_id"],json!({"project_id":id}))]});
    let selection = variant(
        "source",
        "native",
        &[
            "entry_id",
            "name",
            "provider",
            "account_profile",
            "execution_id",
            "inventory_revision",
        ],
        json!({"entry_id":text,"name":name,"provider":text,"account_profile":text,"execution_id":text,"inventory_revision":text}),
    );
    let source = json!({"oneOf":[
        variant("kind","folder",&["path"],json!({"path":text})),
        variant("kind","content",&["skill_md"],json!({"skill_md":text})),
        variant("kind","url",&["url"],json!({"url":text,"pinned_revision":{"type":["string","null"]}})),
        variant("kind","native",&["selection"],json!({"selection":selection}))]});
    let collision = json!({"oneOf":[variant("action","reject",&[],json!({})),
        variant("action","rename",&["name"],json!({"name":name})),
        variant("action","replace",&["skill_id"],json!({"skill_id":id}))]});
    let state = json!({"type":["string","null"],"enum":["auto","manual","off",null],"description":"null resets to inherited state"});
    let actions = json!({"oneOf":[
        variant("action","install",&["source","collision"],json!({"source":source,"collision":collision,"package_id":optional_id})),
        variant("action","update",&["skill_id","source","expected_digest"],json!({"skill_id":id,"source":source,"expected_digest":text})),
        variant("action","remove",&["skill_id"],json!({"skill_id":id})),
        variant("action","set_state",&["skill_id","state"],json!({"skill_id":id,"state":state})),
        variant("action","create_package",&["package_id","name"],json!({"package_id":id,"name":name})),
        variant("action","set_package_source",&["package_id","source_locator","source_revision","upstream_members"],json!({"package_id":id,"source_locator":text,"source_revision":text,"upstream_members":{"type":"object","additionalProperties":id}})),
        variant("action","rename_package",&["package_id","name"],json!({"package_id":id,"name":name})),
        variant("action","regroup",&["skill_id","package_id"],json!({"skill_id":id,"package_id":optional_id})),
        variant("action","remove_package",&["package_id","remove_skills"],json!({"package_id":id,"remove_skills":{"type":"boolean"}})),
        variant("action","set_package_state",&["package_id","state"],json!({"package_id":id,"state":state})),
        variant("action","move",&["skill_id","destination"],json!({"skill_id":id,"destination":scope}))]});
    let mutation = object(
        &["scope", "expected_revision", "actions"],
        json!({"scope":scope,"expected_revision":{"type":"integer","minimum":0},"actions":{"type":"array","minItems":1,"maxItems":256,"items":actions}}),
    );
    let mut schema = object(
        &["action", "mutation", "origin_prompt_id"],
        json!({
        "action":{"type":"string","enum":["preview","publish"]},"mutation":mutation,
        "origin_prompt_id":{"type":"string","minLength":1,"description":"Required on preview AND publish: the current prompt_id from this turn context; never a previous turn."},"review_receipt_id":text}),
    );
    schema["oneOf"] = json!([
        {"properties":{"action":{"enum":["preview"]}},"not":{"required":["review_receipt_id"]}},
        {"properties":{"action":{"enum":["publish"]}},"required":["review_receipt_id"]}]);
    schema["examples"] = json!([{
        "action":"preview", "origin_prompt_id":"current_prompt_id",
        "mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[{
            "action":"install","source":{"kind":"folder","path":"skills/example"},
            "collision":{"action":"reject"}
        }]}
    }]);
    schema
}
