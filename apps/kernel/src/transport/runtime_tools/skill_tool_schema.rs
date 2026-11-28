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
        variant("kind","native",&["selection"],json!({"selection":selection})),
        variant("kind","repository",&["url","revision","path"],json!({"url":text,"revision":text,"path":{"type":"string","description":"skill directory from resolve_repository; empty for a root skill"}}))]});
    let mut update_source = source.clone();
    update_source["oneOf"].as_array_mut().unwrap().push(variant(
        "kind",
        "recorded_folder",
        &[],
        json!({}),
    ));
    let collision = json!({"oneOf":[variant("action","reject",&[],json!({})),
        variant("action","rename",&["name"],json!({"name":name})),
        variant("action","replace",&["skill_id"],json!({"skill_id":id}))]});
    let state = json!({"type":["string","null"],"enum":["auto","manual","off",null],"description":"null resets to inherited state"});
    let actions = json!({"oneOf":[
        variant("action","install",&["source","collision"],json!({"source":source,"collision":collision,"package_id":optional_id})),
        variant("action","update",&["skill_id","source","expected_digest"],json!({"skill_id":id,"source":update_source,"expected_digest":text})),
        variant("action","remove",&["skill_id"],json!({"skill_id":id})),
        variant("action","set_state",&["skill_id","state"],json!({"skill_id":id,"state":state})),
        variant("action","create_package",&["package_id","name"],json!({"package_id":id,"name":name})),
        variant("action","rename_package",&["package_id","name"],json!({"package_id":id,"name":name})),
        variant("action","regroup",&["skill_id","package_id"],json!({"skill_id":id,"package_id":optional_id})),
        variant("action","remove_package",&["package_id","remove_skills"],json!({"package_id":id,"remove_skills":{"type":"boolean"}})),
        variant("action","set_package_state",&["package_id","state"],json!({"package_id":id,"state":state})),
        variant("action","move",&["skill_id","destination"],json!({"skill_id":id,"destination":scope}))]});
    let mutation = object(
        &["scope", "expected_revision", "actions"],
        json!({"scope":scope,"expected_revision":{"type":"integer","minimum":0},"actions":{"type":"array","minItems":1,"maxItems":256,"items":actions}}),
    );
    let mut request = object(
        &["action"],
        json!({
        "action":{"type":"string","enum":["preview","publish","resolve_repository","resolve_repository_update"]},"mutation":mutation,
        "review_receipt_id":text,
        "url":{"type":"string","minLength":1,"description":"resolve_repository: a public https git repository"},
        "package_id":id}),
    );
    let only = |action: &str, required: &[&str]| {
        let absent: Vec<_> = ["mutation", "review_receipt_id", "url", "package_id"]
            .into_iter()
            .filter(|field| !required.contains(field))
            .map(|field| json!({"required":[field]}))
            .collect();
        json!({"properties":{"action":{"enum":[action]}},"required":required,"not":{"anyOf":absent}})
    };
    request["oneOf"] = json!([
        only("preview", &["mutation"]),
        only("publish", &["mutation", "review_receipt_id"]),
        only("resolve_repository", &["url"]),
        only("resolve_repository_update", &["package_id"])
    ]);
    // Providers accept composition inside a property, but some reject it at the tool root.
    let mut schema = object(
        &["origin_prompt_id", "request"],
        json!({
            "origin_prompt_id":{"type":"string","minLength":1,"description":"Required on every action: the current prompt_id from this turn context; never a previous turn."},
            "request":request,
        }),
    );
    schema["examples"] = json!([{
        "origin_prompt_id":"current_prompt_id", "request":{"action":"preview",
        "mutation":{"scope":{"kind":"chariox"},"expected_revision":0,"actions":[{
            "action":"install","source":{"kind":"folder","path":"skills/example"},
            "collision":{"action":"reject"}
        }]}}
    }]);
    schema
}
