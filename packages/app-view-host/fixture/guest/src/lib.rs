//! Backend of the native-open fixture: a note list kept in the App's durable state.
//! `add_note` is the typed effect the page calls; `list_notes` reads what the kernel persisted.
//! `view_actions` and `view_state` are the reserved tools through which the backend serves its own view to an agent.
use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, get_json, registry, state, FutureReader, Guest,
};
use serde_json::{json, Value};

struct Notes;
chariox_app_sdk::bindings::export!(Notes with_types_in chariox_app_sdk::bindings);

/// The signed declarations the packer puts in the manifest; the backend registers the same document.
const UI_ACTIONS: &str = include_str!("../../ui-actions.json");

fn registered_actions() -> Value {
    let mut actions: Value = serde_json::from_str(UI_ACTIONS).expect("the fixture declarations are JSON");
    let actions = actions["uiActions"].take();
    #[cfg(feature = "widened-registration")]
    let actions = {
        let mut actions = actions;
        actions[1]["effect"] = json!("read");
        actions
    };
    actions
}

async fn call(name: &str, input: &str) -> Result<String, AppError> {
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    let record = get_json("notes").await?;
    let mut notes = record
        .as_ref()
        .map(|(value, _)| value.clone())
        .unwrap_or(json!([]));
    match name {
        "list_notes" => Ok(notes.to_string()),
        "view_actions" => Ok(registered_actions().to_string()),
        "view_state" => Ok(json!({ "notes": notes }).to_string()),
        "add_note" => {
            let text = input["text"]
                .as_str()
                .ok_or_else(|| error("INVALID_ARGUMENT", "text is required"))?;
            let list = notes
                .as_array_mut()
                .ok_or_else(|| error("INVALID_STATE", "notes must be an array"))?;
            let note = json!({ "id": list.len() + 1, "text": text });
            list.push(note.clone());
            state::transact(state::Transaction {
                schema_version: 0,
                checks: vec![state::Check {
                    key: "notes".into(),
                    version: record.map(|(_, version)| version),
                }],
                writes: vec![state::Write::Put(("notes".into(), notes.to_string()))],
                occurrences: vec![],
                wakes: vec![],
            })
            .await?;
            Ok(note.to_string())
        }
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

impl Guest for Notes {
    fn register() -> Result<(), AppError> {
        registry::tool("list_notes");
        registry::tool("add_note");
        registry::tool("view_actions");
        registry::tool("view_state");
        Ok(())
    }

    async fn call_tool(
        name: String,
        input: String,
        _: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        cancellable(cancel, call(&name, &input)).await
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
