//! Backend of the native-open fixture: a note list kept in the App's durable state.
//! `add_note` is the typed effect the page calls; `list_notes` reads what the kernel persisted.
//! The App declares its actions once, in the signed `ui-actions.json`; the SDK's `View` serves them,
//! the state and the change feed to an agent.
use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, get_json, registry, state,
    view::{self, View},
    FutureReader, Guest,
};
use serde_json::{json, Value};
use std::sync::OnceLock;

struct Notes;
chariox_app_sdk::bindings::export!(Notes with_types_in chariox_app_sdk::bindings);

/// The signed declarations the packer puts in the manifest.
const UI_ACTIONS: &str = include_str!("../../ui-actions.json");

fn view() -> &'static View {
    static VIEW: OnceLock<View> = OnceLock::new();
    VIEW.get_or_init(|| {
        #[cfg(not(feature = "widened-registration"))]
        let declaration = UI_ACTIONS.to_owned();
        // A backend whose registration widens an action beyond what the signed manifest declares.
        #[cfg(feature = "widened-registration")]
        let declaration = {
            let mut document: Value =
                serde_json::from_str(UI_ACTIONS).expect("the fixture declarations are JSON");
            document["uiActions"][1]["effect"] = json!("read");
            document.to_string()
        };
        View::declare(&declaration)
            .expect("the fixture declarations are valid")
            .with_changes()
    })
}

async fn call(name: &str, input: &str) -> Result<String, AppError> {
    let record = get_json("notes").await?;
    let mut notes = record
        .as_ref()
        .map(|(value, _)| value.clone())
        .unwrap_or(json!([]));
    if let Some(answer) = view()
        .serve(name, input, || async { Ok(json!({ "notes": notes })) })
        .await
    {
        return answer;
    }
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    match name {
        "list_notes" => Ok(notes.to_string()),
        "add_note" => {
            let text = input["text"]
                .as_str()
                .ok_or_else(|| error("INVALID_ARGUMENT", "text is required"))?;
            let list = notes
                .as_array_mut()
                .ok_or_else(|| error("INVALID_STATE", "notes must be an array"))?;
            let note = json!({ "id": list.len() + 1, "text": text });
            list.push(note.clone());
            view::transact_with_change(
                state::Transaction {
                    schema_version: 0,
                    checks: vec![state::Check {
                        key: "notes".into(),
                        version: record.map(|(_, version)| version),
                    }],
                    writes: vec![state::Write::Put(("notes".into(), notes.to_string()))],
                    occurrences: vec![],
                    wakes: vec![],
                },
                json!({ "added": note }),
            )
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
        view().register();
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
