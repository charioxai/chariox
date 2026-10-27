//! MP-08 / MP-10 / MP-11: published MCP fields match the existing typed wire union.
use serde_json::{json, Map, Value};
fn object(properties: Vec<(&str, Value)>) -> Value {
    let required: Vec<_> = properties.iter().map(|(name, _)| *name).collect();
    let properties: Map<_, _> = properties
        .into_iter()
        .map(|(name, value)| (name.into(), value))
        .collect();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn tagged<'a>(tag: &'a str, value: &str, mut properties: Vec<(&'a str, Value)>) -> Value {
    properties.insert(0, (tag, json!({"enum":[value]})));
    object(properties)
}
fn bounded_string(maximum: u32) -> Value {
    json!({"type":"string","minLength":1,"maxLength":maximum})
}
fn coordinate(maximum: u32) -> Value {
    json!({"type":"integer","minimum":0,"maximum":maximum})
}
pub(super) fn tool_schema() -> Value {
    let target = object(vec![
        ("surface_id", bounded_string(128)),
        ("generation", bounded_string(128)),
    ]);
    // Only agent-supported input kinds are advertised. Human-only commands keep
    // their existing typed/native admission; they are not provider capabilities.
    let inputs = vec![
        tagged("kind", "text", vec![("text", bounded_string(128))]),
        tagged("kind", "composition", vec![("text", bounded_string(128))]),
        tagged("kind", "key", vec![("key", bounded_string(128))]),
        tagged(
            "kind",
            "click",
            vec![
                ("x", coordinate(1279)),
                ("y", coordinate(799)),
                ("button", json!({"enum":[1,3]})),
            ],
        ),
        tagged(
            "kind",
            "move",
            vec![("x", coordinate(1279)), ("y", coordinate(799))],
        ),
        tagged(
            "kind",
            "scroll",
            vec![
                ("x", coordinate(1279)),
                ("y", coordinate(799)),
                (
                    "steps",
                    json!({"type":"integer","minimum":-100,"maximum":100}),
                ),
            ],
        ),
    ];
    let mut commands = vec![tagged("op", "start", vec![]), tagged("op", "state", vec![])];
    for op in [
        "installed_applications",
        "snapshot",
        "screenshot",
        "clipboard_read",
    ] {
        commands.push(tagged("op", op, vec![("target", target.clone())]));
    }
    commands.push(tagged("op","launch_application",vec![("target",target.clone()),("application_id",json!({"type":"string","minLength":9,"maxLength":128,"pattern":"^[A-Za-z0-9][A-Za-z0-9_.-]*\\.desktop$"}))]));
    let mut ocr = tagged("op", "ocr", vec![("target", target.clone())]);
    ocr["properties"]["query"] = json!({"type":["string","null"],"minLength":1,"maxLength":4096});
    commands.push(ocr);
    commands.push(tagged(
        "op",
        "input",
        vec![
            ("target", target.clone()),
            ("input", json!({"type":"object","oneOf":inputs})),
        ],
    ));
    commands.push(tagged(
        "op",
        "target_action",
        vec![
            ("target", target),
            ("tree_revision", json!({"type":"integer","minimum":1})),
            ("target_id", bounded_string(128)),
            ("action", bounded_string(128)),
        ],
    ));
    object(vec![("command", json!({"type":"object","oneOf":commands}))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::KernelComputerCommand;
    use serde_json::json;
    fn target() -> Value {
        json!({"surface_id":"desktop-public","generation":"generation-public"})
    }
    fn valid(value: &Value) -> bool {
        jsonschema::JSONSchema::compile(&tool_schema())
            .unwrap()
            .is_valid(value)
    }
    #[test]
    fn mp10_computer_schema_refuses_observer_handles_on_input() {
        let value = json!({"command":{"op":"input","target":target(),"input":{"kind":"key","key":"Alt+F10"},"tree_revision":1,"target_id":"atspi-public"}});
        assert!(serde_json::from_value::<KernelComputerCommand>(value["command"].clone()).is_err());
        assert!(
            !valid(&value),
            "MP-10: official provider schema must refuse the shape the typed parser rejects"
        );
    }
    #[test]
    fn mp10_computer_schema_requires_each_operation_and_input_fields() {
        for command in [
            json!({"op":"input","input":{"kind":"key","key":"Return"}}),
            json!({"op":"input","target":target(),"input":{"kind":"text","key":"Return"}}),
            json!({"op":"start","target":target()}),
            json!({"op":"snapshot"}),
        ] {
            assert!(serde_json::from_value::<KernelComputerCommand>(command.clone()).is_err());
            assert!(
                !valid(&json!({"command":command})),
                "MP-10: parser refusal must be represented in the published schema"
            );
        }
    }
    #[test]
    fn mp10_computer_schema_retains_agent_commands() {
        let mut commands = vec![
            json!({"op":"start"}),
            json!({"op":"state"}),
            json!({"op":"launch_application","target":target(),"application_id":"org.xfce.mousepad.desktop"}),
            json!({"op":"ocr","target":target(),"query":"public"}),
            json!({"op":"target_action","target":target(),"tree_revision":1,"target_id":"atspi-public","action":"click"}),
        ];
        for op in [
            "installed_applications",
            "snapshot",
            "screenshot",
            "clipboard_read",
        ] {
            commands.push(json!({"op":op,"target":target()}));
        }
        for input in [
            json!({"kind":"text","text":"public"}),
            json!({"kind":"composition","text":"public"}),
            json!({"kind":"key","key":"Alt+F10"}),
            json!({"kind":"click","x":1,"y":1,"button":1}),
            json!({"kind":"move","x":1,"y":1}),
            json!({"kind":"scroll","x":1,"y":1,"steps":1}),
        ] {
            commands.push(json!({"op":"input","target":target(),"input":input}));
        }
        for command in commands {
            assert!(serde_json::from_value::<KernelComputerCommand>(command.clone()).is_ok());
            assert!(valid(&json!({"command":command})));
        }
    }
}
