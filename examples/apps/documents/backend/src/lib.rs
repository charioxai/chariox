//! Documents: Markdown and HTML documents the person and their agents write together. Every
//! edit states the revision it started from, so a human and an agent never silently overwrite
//! each other; the last ten revisions of each document are kept and restorable.

use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, registry,
    view::View,
    FutureReader, Guest,
};
use serde_json::{json, Value};

mod draft;
mod index;
mod model;
mod store;
mod transfer;

struct Documents;
chariox_app_sdk::bindings::export!(Documents with_types_in chariox_app_sdk::bindings);

/// The App's view: the declared actions, the list of documents as the state, and the feed of
/// saved and deleted documents that the window follows.
fn view() -> Result<View, AppError> {
    Ok(View::declare(include_str!("../../schemas/ui-actions.json"))?.with_changes())
}

/// Every tool is also a declared action of the view.
const TOOLS: [&str; 12] = [
    "list_documents",
    "read_document",
    "create_document",
    "update_document",
    "restore_version",
    "delete_document",
    "import_documents",
    "export_document",
    "snapshot_documents",
    "save_draft",
    "read_draft",
    "discard_draft",
];

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, AppError> {
    input[key]
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", format!("{key} is required")))
}

fn optional(input: &Value, key: &str) -> Option<String> {
    input[key].as_str().map(str::to_owned)
}

fn revision(input: &Value, key: &str) -> Result<u64, AppError> {
    input[key]
        .as_u64()
        .ok_or_else(|| error("INVALID_ARGUMENT", format!("{key} is required")))
}

async fn call(name: &str, input: &Value) -> Result<Value, AppError> {
    match name {
        "list_documents" => {
            let folder = input["folder"].as_str();
            let (docs, _) = index::load().await?;
            let listed: Vec<Value> = docs
                .iter()
                .filter(|doc| folder.is_none_or(|folder| doc.folder == folder))
                .map(model::Doc::summary)
                .collect();
            Ok(json!({"documents": listed}))
        }
        "read_document" => {
            let (docs, _) = index::load().await?;
            let doc = model::find(&docs, text(input, "id")?)?;
            let at = input["revision"].as_u64().unwrap_or(doc.revision);
            let mut read = doc.summary();
            read["content"] = json!(store::read(doc, at)?);
            Ok(read)
        }
        "create_document" => {
            let kind = input["kind"]
                .as_str()
                .map_or(Ok(model::Kind::Markdown), model::Kind::parse)?;
            let new = model::NewDoc {
                // One id for every retry, so a lost compare-and-set leaves no stray folder.
                id: store::nonce(),
                title: text(input, "title")?.to_owned(),
                kind,
                folder: optional(input, "folder").unwrap_or_default(),
                content: optional(input, "content").unwrap_or_default(),
            };
            index::change(|docs| model::create(docs, new.clone(), &store::stamp())).await
        }
        "update_document" => {
            let (id, expected) = (text(input, "id")?, revision(input, "expected_revision")?);
            index::change(|docs| {
                let change = model::Revision {
                    content: optional(input, "content"),
                    title: optional(input, "title"),
                    folder: optional(input, "folder"),
                };
                model::revise(docs, id, expected, change, &store::stamp())
            })
            .await
        }
        // Restores content only; a rename or move revision restores nothing visible.
        "restore_version" => {
            let (id, earlier) = (text(input, "id")?, revision(input, "revision")?);
            let expected = revision(input, "expected_revision")?;
            index::change(|docs| {
                let doc = model::find(docs, id)?;
                let content = store::read(doc, earlier)?;
                let change = model::Revision {
                    content: Some(content),
                    title: None,
                    folder: None,
                };
                model::revise(docs, id, expected, change, &store::stamp())
            })
            .await
        }
        "delete_document" => {
            let id = text(input, "id")?;
            index::change(|docs| model::remove(docs, id)).await
        }
        "import_documents" => {
            let folder = input["folder"].as_str().unwrap_or("");
            match input["files"].as_array() {
                Some(files) => transfer::import_uploaded(folder, files).await,
                None => transfer::import(folder, input["importId"].as_str()).await,
            }
        }
        "export_document" => transfer::export(text(input, "id")?).await,
        "snapshot_documents" => {
            transfer::snapshot(input["name"].as_str().unwrap_or("documents")).await
        }
        "save_draft" => {
            let (docs, _) = index::load().await?;
            let (id, base) = (text(input, "id")?, revision(input, "base_revision")?);
            draft::save(&docs, id, base, input).await
        }
        "read_draft" => draft::read(&index::load().await?.0, input["id"].as_str()),
        "discard_draft" => draft::discard(text(input, "id")?),
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

async fn view_state() -> Result<Value, AppError> {
    call("list_documents", &json!({})).await
}

impl Guest for Documents {
    fn register() -> Result<(), AppError> {
        let view = view()?;
        view.register();
        TOOLS.iter().for_each(|name| registry::tool(name));
        Ok(())
    }
    async fn call_tool(
        name: String,
        input: String,
        _: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        cancellable(cancel, async {
            if let Some(answer) = view()?.serve(&name, &input, view_state).await {
                return answer;
            }
            let input: Value = serde_json::from_str(&input)
                .map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
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
    fn the_manifest_lists_what_the_backend_registers_and_every_tool_is_a_declared_action() {
        let tools = document(include_str!("../../schemas/tools.json"), "tools");
        let actions = document(include_str!("../../schemas/ui-actions.json"), "uiActions");
        let listed: BTreeSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let registered: BTreeSet<&str> = TOOLS
            .iter()
            .chain(&["view_actions", "view_state", "view_changes"])
            .copied()
            .collect();
        assert_eq!(
            listed, registered,
            "schemas/tools.json is what register() registers"
        );
        assert_eq!(actions.len(), TOOLS.len());
        for action in &actions {
            let name = action["name"].as_str().unwrap();
            let tool = tools.iter().find(|t| t["name"] == action["name"]).unwrap();
            assert!(TOOLS.contains(&name), "the backend runs {name}");
            assert_eq!(tool["inputSchema"], action["inputSchema"], "{name}");
            assert_eq!(tool["description"], action["description"], "{name}");
        }
    }

    #[test]
    fn an_agent_asks_the_person_before_deleting_and_nothing_else_asks() {
        let actions = document(include_str!("../../schemas/ui-actions.json"), "uiActions");
        for action in &actions {
            let name = action["name"].as_str().unwrap();
            let asks = action["effect"] == "approval";
            assert_eq!(asks, name == "delete_document", "{name}");
            assert_eq!(
                action.get("approvalReason").is_some(),
                asks,
                "{name} states why only when it asks"
            );
        }
    }
}
