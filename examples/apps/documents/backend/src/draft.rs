//! The person's unsaved edits of open documents. The window's sandbox keeps nothing across a
//! replacement (an App update, a reconnect, a reload), so a window hands its edit to the App and
//! asks for it again when it starts.
//!
//! A draft belongs to one editor, a window that edits a document: it is kept per (document, editor),
//! so two windows on the same document each have their own, and `save_draft` and `discard_draft`
//! only ever write or remove the file of the editor that sent them. A replaced window's successor
//! is a new editor with no memory of its predecessor, so it asks `claim_draft`: the one place a draft
//! changes hands. It keeps what the editor already has, and otherwise moves the most recently kept
//! draft of another editor to itself (own copy written first, then the source removed) and a draft
//! whose document was deleted is dropped, since nothing is left to come back to. Each draft records
//! the revision its edit started from, so a save after someone else's change is still refused as a
//! conflict, never merged.

use crate::{model, store};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

const FOLDER: &str = "drafts";
const MAX_ID: usize = 64;
/// Drafts the App keeps at once, so windows that were closed with unsaved edits cannot fill the disk.
const MAX_DRAFTS: usize = 32;

fn plain(what: &str, value: &str) -> Result<(), AppError> {
    let plain = !value.is_empty()
        && value.len() <= MAX_ID
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if plain {
        Ok(())
    } else {
        Err(error("INVALID_ARGUMENT", format!("{what} is not an id")))
    }
}

/// Where the draft of `editor` for document `id` is kept. Neither id contains the `.` between them,
/// so no two (document, editor) pairs share a file and no id can name another path.
fn path_of(id: &str, editor: &str) -> Result<String, AppError> {
    plain("id", id)?;
    plain("editor", editor)?;
    Ok(format!("{FOLDER}/{id}.{editor}.json"))
}

/// The draft as the window sent it, checked by the same rules as a save.
pub fn parse_new(
    docs: &[model::Doc],
    editor: &str,
    id: &str,
    base_revision: u64,
    edit: &Value,
    saved_at_ms: u64,
) -> Result<String, AppError> {
    plain("editor", editor)?;
    model::find(docs, id)?;
    let field = |key: &str| edit[key].as_str().unwrap_or_default();
    model::check_name("title", field("title"), true)?;
    model::check_name("folder", field("folder"), true)?;
    model::check_content(field("content"))?;
    Ok(json!({
        "editor": editor,
        "id": id,
        "base_revision": base_revision,
        "saved_at_ms": saved_at_ms,
        "title": field("title"),
        "folder": field("folder"),
        "content": field("content"),
    })
    .to_string())
}

/// A kept draft, read back.
struct Kept {
    editor: String,
    id: String,
    saved_at_ms: u64,
    value: Value,
}

impl Kept {
    fn parse(text: &str) -> Result<Self, AppError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| error("INVALID_STATE", e.to_string()))?;
        let name = |key: &str| {
            value[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| error("INVALID_STATE", format!("a draft has no {key}")))
        };
        Ok(Self {
            editor: name("editor")?,
            id: name("id")?,
            saved_at_ms: value["saved_at_ms"].as_u64().unwrap_or(0),
            value,
        })
    }

    /// What the window is handed: the edit, not who kept it or when.
    fn edit(&self) -> Value {
        let mut edit = self.value.clone();
        if let Some(fields) = edit.as_object_mut() {
            fields.remove("saved_at_ms");
            fields.remove("editor");
        }
        edit
    }

    fn path(&self) -> Result<String, AppError> {
        path_of(&self.id, &self.editor)
    }
}

/// What `claim_draft` does for an editor.
pub enum Claim {
    /// The editor already has a draft; nothing changes hands.
    Keep(Value),
    /// The most recently kept draft of another editor moves to this one.
    Take {
        text: String,
        source: String,
        edit: Value,
    },
    Nothing,
}

/// What the kept drafts say to an editor that asks for one, and the files of drafts whose document
/// no longer exists.
pub fn choose(
    docs: &[model::Doc],
    editor: &str,
    texts: &[String],
    saved_at_ms: u64,
) -> Result<(Claim, Vec<String>), AppError> {
    plain("editor", editor)?;
    let mut gone = Vec::new();
    let mut alive = Vec::new();
    for text in texts {
        let kept = Kept::parse(text)?;
        if docs.iter().any(|doc| doc.id == kept.id) {
            alive.push(kept);
        } else {
            gone.push(kept.path()?);
        }
    }
    let latest = |own: bool| {
        alive
            .iter()
            .filter(|kept| (kept.editor == editor) == own)
            .max_by_key(|kept| kept.saved_at_ms)
    };
    let claim = if let Some(own) = latest(true) {
        Claim::Keep(own.edit())
    } else if let Some(other) = latest(false) {
        let mut moved = other.value.clone();
        moved["editor"] = json!(editor);
        moved["saved_at_ms"] = json!(saved_at_ms);
        Claim::Take {
            text: moved.to_string(),
            source: other.path()?,
            edit: other.edit(),
        }
    } else {
        Claim::Nothing
    };
    Ok((claim, gone))
}

pub async fn save(
    docs: &[model::Doc],
    editor: &str,
    id: &str,
    base_revision: u64,
    edit: &Value,
) -> Result<Value, AppError> {
    let text = parse_new(docs, editor, id, base_revision, edit, store::now_ms())?;
    let path = path_of(id, editor)?;
    let kept = store::private_files(FOLDER)?;
    if kept.len() >= MAX_DRAFTS && !kept.contains(&path) {
        return Err(error(
            "INVALID_STATE",
            "too many unsaved drafts are kept; save or discard one first",
        ));
    }
    store::replace(&path, text.into_bytes()).await?;
    Ok(json!({"saved": true}))
}

/// The draft the editor asks for when its window starts; see `choose`.
pub async fn claim(docs: &[model::Doc], editor: &str) -> Result<Value, AppError> {
    let texts = store::read_private_folder(FOLDER)?;
    let (claim, gone) = choose(docs, editor, &texts, store::now_ms())?;
    for path in gone {
        store::remove_private(&path)?;
    }
    let draft = match claim {
        Claim::Keep(edit) => Some(edit),
        Claim::Take { text, source, edit } => {
            let own = path_of(edit["id"].as_str().unwrap_or_default(), editor)?;
            store::replace(&own, text.into_bytes()).await?;
            store::remove_private(&source)?;
            Some(edit)
        }
        Claim::Nothing => None,
    };
    Ok(json!({"draft": draft}))
}

/// Removes the draft of `editor` for `id`, and no other.
pub fn discard(editor: &str, id: &str) -> Result<Value, AppError> {
    store::remove_private(&path_of(id, editor)?)?;
    Ok(json!({"discarded": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs() -> Vec<model::Doc> {
        let mut docs = Vec::new();
        for id in ["d1", "d2"] {
            let plan = model::create(
                &docs,
                model::NewDoc {
                    id: id.into(),
                    title: format!("Notes {id}"),
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
            docs = plan.docs;
        }
        docs
    }

    fn edit(content: &str) -> Value {
        json!({"title": "t", "folder": "", "content": content})
    }

    fn kept(editor: &str, id: &str, content: &str, at: u64) -> String {
        parse_new(&docs(), editor, id, 1, &edit(content), at).unwrap()
    }

    #[test]
    fn a_draft_keeps_the_revision_it_started_from_and_every_unsaved_field() {
        let docs = docs();
        let edit = json!({"title": "Notes 2", "folder": "work", "content": "unsaved"});
        let text = parse_new(&docs, "w1", "d1", 1, &edit, 5).unwrap();
        let draft = Kept::parse(&text).unwrap().edit();
        assert_eq!(draft["base_revision"], 1);
        assert_eq!(draft["title"], "Notes 2");
        assert_eq!(draft["folder"], "work");
        assert_eq!(draft["content"], "unsaved");
        assert!(draft.get("editor").is_none() && draft.get("saved_at_ms").is_none());
    }

    #[test]
    fn a_draft_of_a_missing_document_is_refused_and_one_whose_document_was_deleted_is_dropped() {
        let docs = docs();
        assert!(parse_new(&docs, "w1", "missing", 1, &edit("c"), 5).is_err());
        let text = kept("w1", "d1", "c", 5);
        let (claim, gone) = choose(&[], "w2", &[text], 9).unwrap();
        assert!(matches!(claim, Claim::Nothing));
        assert_eq!(gone, ["drafts/d1.w1.json"]);
    }

    #[test]
    fn a_draft_is_held_to_the_size_of_a_document() {
        let big = edit(&"x".repeat(model::MAX_CONTENT + 1));
        assert!(parse_new(&docs(), "w1", "d1", 1, &big, 5).is_err());
    }

    #[test]
    fn each_document_and_editor_pair_has_its_own_draft_file_and_no_id_can_name_another_path() {
        assert_eq!(path_of("d1", "w1").unwrap(), "drafts/d1.w1.json");
        let files = [
            path_of("d1", "w1").unwrap(),
            path_of("d1", "w2").unwrap(),
            path_of("d2", "w1").unwrap(),
            path_of("d1-w", "1").unwrap(),
            path_of("d1", "w-1").unwrap(),
        ];
        let unique: std::collections::BTreeSet<_> = files.iter().collect();
        assert_eq!(unique.len(), files.len(), "{files:?}");
        for bad in ["", "../x", "a/b", "a.b", &"x".repeat(MAX_ID + 1)] {
            assert!(path_of(bad, "w1").is_err(), "id {bad:?}");
            assert!(path_of("d1", bad).is_err(), "editor {bad:?}");
            assert!(
                parse_new(&docs(), bad, "d1", 1, &edit("c"), 5).is_err(),
                "editor {bad:?}"
            );
        }
    }

    #[test]
    fn two_editors_of_one_document_keep_two_drafts_and_each_claims_only_its_own() {
        let texts = vec![
            kept("w1", "d1", "first window", 5),
            kept("w2", "d1", "second window", 6),
        ];
        for (editor, content) in [("w1", "first window"), ("w2", "second window")] {
            let (claim, gone) = choose(&docs(), editor, &texts, 9).unwrap();
            let Claim::Keep(draft) = claim else {
                panic!("{editor} has a draft of its own");
            };
            assert_eq!(draft["content"], content);
            assert!(gone.is_empty());
        }
    }

    #[test]
    fn a_new_editor_is_handed_the_latest_draft_of_another_editor_and_the_source_is_named_to_move() {
        let texts = vec![
            kept("w1", "d1", "earlier", 5),
            kept("w2", "d2", "later", 9),
            kept("w3", "d1", "earliest", 1),
        ];
        let (claim, gone) = choose(&docs(), "w4", &texts, 20).unwrap();
        let Claim::Take { text, source, edit } = claim else {
            panic!("a draft changes hands");
        };
        assert_eq!(edit["content"], "later");
        assert_eq!(edit["id"], "d2");
        assert_eq!(source, "drafts/d2.w2.json");
        let moved = Kept::parse(&text).unwrap();
        assert_eq!((moved.editor.as_str(), moved.saved_at_ms), ("w4", 20));
        assert_eq!(moved.path().unwrap(), "drafts/d2.w4.json");
        assert!(gone.is_empty());
    }

    #[test]
    fn an_editor_with_a_draft_takes_nothing_from_another_and_one_with_nothing_to_find_gets_nothing()
    {
        let texts = vec![
            kept("w1", "d1", "mine", 1),
            kept("w2", "d2", "newer, not mine", 9),
        ];
        let (claim, _) = choose(&docs(), "w1", &texts, 20).unwrap();
        let Claim::Keep(draft) = claim else {
            panic!("its own draft is kept");
        };
        assert_eq!(draft["content"], "mine");
        let (claim, _) = choose(&docs(), "w9", &[], 20).unwrap();
        assert!(matches!(claim, Claim::Nothing));
    }
}
