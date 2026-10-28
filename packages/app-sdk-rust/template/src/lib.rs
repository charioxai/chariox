use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, get_json, registry, state, FutureReader, Guest,
};
use serde_json::{json, Value};

struct Todo;
chariox_app_sdk::bindings::export!(Todo with_types_in chariox_app_sdk::bindings);

async fn call(name: &str, input: &str) -> Result<String, AppError> {
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    let record = get_json("todos").await?;
    let mut todos = record.as_ref().map(|(v, _)| v.clone()).unwrap_or(json!([]));
    match name {
        "list" => Ok(todos.to_string()),
        "add" => {
            let title = input["title"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| error("INVALID_ARGUMENT", "title is required"))?;
            todos
                .as_array_mut()
                .ok_or_else(|| error("INVALID_STATE", "todos must be an array"))?
                .push(json!({ "title": title }));
            state::transact(state::Transaction {
                schema_version: 0,
                checks: vec![state::Check {
                    key: "todos".into(),
                    version: record.map(|(_, v)| v),
                }],
                writes: vec![state::Write::Put(("todos".into(), todos.to_string()))],
                occurrences: vec![],
                wakes: vec![],
            })
            .await?;
            Ok(todos.to_string())
        }
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

impl Guest for Todo {
    fn register() -> Result<(), AppError> {
        registry::tool("list");
        registry::tool("add");
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
