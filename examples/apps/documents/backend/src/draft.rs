//! The person's unsaved edit of the open document. The window's sandbox keeps nothing across a
//! replacement (an App update, a reconnect, a reload), so the window hands the edit to the App and
//! asks for it again when it starts. The window has no identity of its own that survives a replacement,
//! so a draft belongs to the document being edited: one draft per document, and a window that saves
//! or gives up its edit discards that document's draft and no other. It records the revision the edit
//! started from, so a save after someone else's change is still refused as a conflict, never merged.

use crate::{model, store};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

const FOLDER: &str = "drafts";
const MAX_ID: usize = 64;

/// Where the draft of a document is kept. The id names a file, so it must look like one id.
fn path_of(id: &str) -> Result<String, AppError> {
    let plain = !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !plain {
        return Err(error("INVALID_ARGUMENT", "id is not a document id"));
    }
    Ok(format!("{FOLDER}/{id}.json"))
}

/// The draft as the window sent it, checked by the same rules as a save.
pub fn parse_new(
    docs: &[model::Doc],
    id: &str,
    base_revision: u64,
    edit: &Value,
    saved_at_ms: u64,
) -> Result<String, AppError> {
    model::find(docs, id)?;
    let field = |key: &str| edit[key].as_str().unwrap_or_default();
    model::check_name("title", field("title"), true)?;
    model::check_name("folder", field("folder"), true)?;
    model::check_content(field("content"))?;
    Ok(json!({
        "id": id,
        "base_revision": base_revision,
        "saved_at_ms": saved_at_ms,
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
    Ok(alive.then(|| without_stamp(draft)))
}

/// The most recently saved of the kept drafts whose document still exists: what a window that starts
/// with no document open is handed.
pub fn parse_latest(docs: &[model::Doc], texts: &[String]) -> Result<Option<Value>, AppError> {
    let mut latest: Option<(u64, Value)> = None;
    for text in texts {
        let saved_at = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|draft| draft["saved_at_ms"].as_u64())
            .unwrap_or(0);
        if let Some(draft) = parse_kept(docs, text)? {
            if latest.as_ref().is_none_or(|(at, _)| saved_at > *at) {
                latest = Some((saved_at, draft));
            }
        }
    }
    Ok(latest.map(|(_, draft)| draft))
}

fn without_stamp(mut draft: Value) -> Value {
    if let Some(fields) = draft.as_object_mut() {
        fields.remove("saved_at_ms");
    }
    draft
}

pub async fn save(
    docs: &[model::Doc],
    id: &str,
    base_revision: u64,
    edit: &Value,
) -> Result<Value, AppError> {
    let text = parse_new(docs, id, base_revision, edit, store::now_ms())?;
    store::replace(&path_of(id)?, text.into_bytes()).await?;
    Ok(json!({"saved": true}))
}

/// The draft of `id`, or with no id the most recently saved one.
pub fn read(docs: &[model::Doc], id: Option<&str>) -> Result<Value, AppError> {
    let kept = match id {
        Some(id) => match store::read_private(&path_of(id)?)? {
            Some(text) => parse_kept(docs, &text)?,
            None => None,
        },
        None => parse_latest(docs, &store::read_private_folder(FOLDER)?)?,
    };
    Ok(json!({"draft": kept}))
}

pub fn discard(id: &str) -> Result<Value, AppError> {
    store::remove_private(&path_of(id)?)?;
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
        let text = parse_new(&docs, "d1", 1, &edit, 5).unwrap();
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
        assert!(parse_new(&docs, "missing", 1, &edit, 5).is_err());
        let text = parse_new(&docs, "d1", 1, &edit, 5).unwrap();
        assert_eq!(parse_kept(&[], &text).unwrap(), None);
    }

    #[test]
    fn a_draft_is_held_to_the_size_of_a_document() {
        let docs = docs();
        let big =
            json!({"title": "t", "folder": "", "content": "x".repeat(model::MAX_CONTENT + 1)});
        assert!(parse_new(&docs, "d1", 1, &big, 5).is_err());
    }

    #[test]
    fn each_document_has_its_own_draft_file_and_an_id_cannot_name_another_path() {
        assert_eq!(path_of("d1").unwrap(), "drafts/d1.json");
        assert_ne!(path_of("d1").unwrap(), path_of("d2").unwrap());
        for bad in ["", "../x", "a/b", "a.json", &"x".repeat(MAX_ID + 1)] {
            assert!(path_of(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_window_with_no_document_open_is_handed_the_latest_draft_of_a_document_that_exists() {
        let docs = docs();
        let edit = |content: &str| json!({"title": "t", "folder": "", "content": content});
        let earlier = parse_new(&docs, "d1", 1, &edit("earlier"), 5).unwrap();
        let later = parse_new(&docs, "d1", 1, &edit("later"), 9).unwrap();
        let deleted = json!({"id": "gone", "saved_at_ms": 99, "content": "newest"}).to_string();
        let texts = vec![earlier, deleted, later];
        let kept = parse_latest(&docs, &texts).unwrap().unwrap();
        assert_eq!(kept["content"], "later");
        assert!(kept.get("saved_at_ms").is_none());
        assert_eq!(parse_latest(&docs, &[]).unwrap(), None);
    }
}
