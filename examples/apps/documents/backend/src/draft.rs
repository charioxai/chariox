//! The person's unsaved edit of the open document. The window's sandbox keeps nothing across a
//! replacement (an App update, a reconnect, a reload), so the window hands the edit to the App and
//! asks for it again when it starts. One draft is kept: the one document the window has open.
//! It records the revision the edit started from, so a save after someone else's change is still
//! refused as a conflict, never merged.

use crate::{model, store};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

const PATH: &str = "draft.json";

/// The draft as the window sent it, checked by the same rules as a save.
pub fn parse_new(
    docs: &[model::Doc],
    id: &str,
    base_revision: u64,
    edit: &Value,
) -> Result<String, AppError> {
    model::find(docs, id)?;
    let field = |key: &str| edit[key].as_str().unwrap_or_default();
    model::check_name("title", field("title"), true)?;
    model::check_name("folder", field("folder"), true)?;
    model::check_content(field("content"))?;
    Ok(json!({
        "id": id,
        "base_revision": base_revision,
        "title": field("title"),
        "folder": field("folder"),
        "content": field("content"),
    })
    .to_string())
}

/// The kept draft when its document still exists.
pub fn parse_kept(docs: &[model::Doc], text: &str) -> Result<Option<Value>, AppError> {
    let draft: Value =
        serde_json::from_str(text).map_err(|e| error("INVALID_STATE", e.to_string()))?;
    let alive = draft["id"]
        .as_str()
        .is_some_and(|id| docs.iter().any(|doc| doc.id == id));
    Ok(alive.then_some(draft))
}

pub async fn save(
    docs: &[model::Doc],
    id: &str,
    base_revision: u64,
    edit: &Value,
) -> Result<Value, AppError> {
    let text = parse_new(docs, id, base_revision, edit)?;
    store::replace(PATH, text.into_bytes()).await?;
    Ok(json!({"saved": true}))
}

pub fn read(docs: &[model::Doc]) -> Result<Value, AppError> {
    let kept = match store::read_private(PATH)? {
        Some(text) => parse_kept(docs, &text)?,
        None => None,
    };
    Ok(json!({"draft": kept}))
}

pub fn discard() -> Result<Value, AppError> {
    store::remove_private(PATH)?;
    Ok(json!({"discarded": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs() -> Vec<model::Doc> {
        let plan = model::create(
            &[],
            model::NewDoc {
                id: "d1".into(),
                title: "Notes".into(),
                kind: model::Kind::Markdown,
                folder: String::new(),
                content: "saved".into(),
            },
            &model::Stamp {
                now_ms: 1,
                nonce: "n1".into(),
            },
        )
        .unwrap();
        plan.docs
    }

    #[test]
    fn a_draft_keeps_the_revision_it_started_from_and_every_unsaved_field() {
        let docs = docs();
        let edit = json!({"title": "Notes 2", "folder": "work", "content": "unsaved"});
        let text = parse_new(&docs, "d1", 1, &edit).unwrap();
        let kept = parse_kept(&docs, &text).unwrap().unwrap();
        assert_eq!(kept["base_revision"], 1);
        assert_eq!(kept["title"], "Notes 2");
        assert_eq!(kept["folder"], "work");
        assert_eq!(kept["content"], "unsaved");
    }

    #[test]
    fn a_draft_of_a_missing_document_is_refused_and_one_whose_document_was_deleted_is_not_returned()
    {
        let docs = docs();
        let edit = json!({"title": "t", "folder": "", "content": "c"});
        assert!(parse_new(&docs, "missing", 1, &edit).is_err());
        let text = parse_new(&docs, "d1", 1, &edit).unwrap();
        assert_eq!(parse_kept(&[], &text).unwrap(), None);
    }

    #[test]
    fn a_draft_is_held_to_the_size_of_a_document() {
        let docs = docs();
        let big =
            json!({"title": "t", "folder": "", "content": "x".repeat(model::MAX_CONTENT + 1)});
        assert!(parse_new(&docs, "d1", 1, &big).is_err());
    }
}
