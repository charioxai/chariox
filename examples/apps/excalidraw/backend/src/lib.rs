use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, registry,
    view::View,
    FutureReader, Guest,
};
use serde_json::{json, Value};

mod actions;
mod binding;
mod diagram;
mod draw;
mod element;
mod export_cleanup;
mod file_io;
mod label;
mod saved;
mod scene;
mod stamp;
mod text;
mod update;

#[cfg(test)]
mod golden;

struct Excalidraw;
chariox_app_sdk::bindings::export!(Excalidraw with_types_in chariox_app_sdk::bindings);

/// The App's view: the actions of the signed declarations, served from the saved scene with no
/// page, and a change feed that the window and the agent's actions both write.
fn view() -> Result<View, AppError> {
    Ok(View::declare(include_str!("../../schemas/ui-actions.json"))?.with_changes())
}

/// The tools that are not declared actions of the view.
const OWN_TOOLS: [&str; 5] = [
    "load_scene",
    "save_scene",
    "write_file",
    "read_file",
    "pick_file",
];

/// The declared actions that move files; the rest of the declared actions run on the saved scene.
const FILE_ACTIONS: [&str; 3] = ["save_file", "open_file", "export_file"];

/// The declared actions the backend runs on the saved scene.
const DIAGRAM_ACTIONS: [&str; 7] = [
    "scene_summary",
    "select_elements",
    "delete_elements",
    "group_elements",
    "add_elements",
    "connect_elements",
    "update_elements",
];

fn version_of(value: &Value) -> Result<Option<u64>, AppError> {
    match value {
        Value::Null => Ok(None),
        Value::String(version) => version
            .parse::<u64>()
            .map(Some)
            .map_err(|_| error("INVALID_ARGUMENT", "version must be a decimal string")),
        _ => Err(error(
            "INVALID_ARGUMENT",
            "version must be a string or null",
        )),
    }
}

async fn call(name: &str, input: &str) -> Result<Value, AppError> {
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    match name {
        "load_scene" => {
            let saved = saved::load().await?;
            Ok(match saved.version {
                Some(version) => json!({"scene": saved.scene, "version": version.to_string()}),
                None => json!({"scene": null, "version": null}),
            })
        }
        "save_scene" => {
            let scene = scene::parse(&input["scene"].to_string())?;
            let version = saved::save(
                &scene,
                version_of(&input["version"])?,
                json!({"tool": "save_scene"}),
            )
            .await?;
            Ok(json!({"version": version.to_string()}))
        }
        "write_file" => file_io::write(&input).await,
        "read_file" => file_io::read(&input).await,
        "pick_file" => file_io::pick().await,
        "save_file" => diagram::save_file(&input).await,
        "open_file" => diagram::open_file(&input).await,
        "export_file" => diagram::export_file(&input).await,
        name if DIAGRAM_ACTIONS.contains(&name) => diagram::run(name, &input).await,
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

impl Guest for Excalidraw {
    fn register() -> Result<(), AppError> {
        let view = view()?;
        view.register();
        for name in OWN_TOOLS
            .iter()
            .chain(&DIAGRAM_ACTIONS)
            .chain(&FILE_ACTIONS)
        {
            registry::tool(name);
        }
        Ok(())
    }
    async fn call_tool(
        name: String,
        input: String,
        _: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        cancellable(cancel, async {
            if let Some(answer) = view()?.serve(&name, &input, saved::view_state).await {
                return answer;
            }
            Ok(call(&name, &input).await?.to_string())
        })
        .await
    }
    async fn deliver_event(
        _: String,
        _: String,
        _: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        Err(error("EVENT_NOT_FOUND", "no incoming events declared"))
    }
    async fn lifecycle(
        _: String,
        _: Option<String>,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<Option<String>, AppError> {
        Ok(None)
    }
    async fn deliver_wake(
        _: Wake,
        _: bool,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<(), AppError> {
        Err(error("WAKE_NOT_FOUND", "no wakes declared"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn document(text: &str, key: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(text).unwrap()[key]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn the_manifest_lists_the_tools_the_backend_registers_and_the_declared_actions_it_runs() {
        let tools = document(include_str!("../../schemas/tools.json"), "tools");
        let actions = document(include_str!("../../schemas/ui-actions.json"), "uiActions");
        let listed: BTreeSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let registered: BTreeSet<&str> = OWN_TOOLS
            .iter()
            .chain(&DIAGRAM_ACTIONS)
            .chain(&FILE_ACTIONS)
            .chain(&["view_actions", "view_state", "view_changes"])
            .copied()
            .collect();
        assert_eq!(
            listed, registered,
            "schemas/tools.json is what register() registers"
        );
        for action in &actions {
            let name = action["name"].as_str().unwrap();
            assert!(
                DIAGRAM_ACTIONS.contains(&name) || FILE_ACTIONS.contains(&name),
                "the backend runs {name}"
            );
            let tool = tools.iter().find(|t| t["name"] == action["name"]).unwrap();
            assert_eq!(tool["inputSchema"], action["inputSchema"], "{name}");
            assert_eq!(tool["description"], action["description"], "{name}");
        }
        assert_eq!(actions.len(), DIAGRAM_ACTIONS.len() + FILE_ACTIONS.len());
    }
}
