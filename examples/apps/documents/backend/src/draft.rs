//! The person's unsaved edits of open documents. The window's sandbox keeps nothing across a
//! replacement (an App update, a reconnect, a reload), so a window hands its edit to the App and
//! asks for it again when it starts.
//!
//! A draft belongs to one editor, a window that edits a document: it is kept per (document, editor),
//! so two windows on the same document each have their own. A draft is written and removed only by
//! the editor it belongs to (`save_draft`, `discard_draft`) or by the age sweep once it is older than
//! a day: nothing ever moves or deletes another editor's draft, because that editor may still be open
//! and typing. A replaced window's successor is a new editor with no memory of its predecessor, so it
//! asks `claim_draft`: it keeps what the editor already has, and otherwise is handed a COPY of the most
//! recently kept draft of another editor, written under its own name; the source stays. The cost is
//! that an edit one window gives up can come back in a later window until it ages out, a duplicate
//! the person can discard, never a lost edit. Each draft records the revision its edit started from,
//! so a save after someone else's change is still refused as a conflict, never merged.

use crate::{model, store};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

const FOLDER: &str = "drafts";
const MAX_ID: usize = 64;
/// Drafts the App keeps at once, so windows that were closed with unsaved edits cannot fill the disk.
const MAX_DRAFTS: usize = 32;
/// A draft nobody has written for this long is dropped; the one removal that is not its editor's own.
const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

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

    fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.saved_at_ms) > MAX_AGE_MS
    }
}

/// What `claim_draft` hands an editor.
#[derive(Clone)]
pub enum Claim {
    /// The editor already has a draft; nothing is written.
    Keep(Value),
    /// The most recently kept draft of another editor, copied to a file of this editor's own. The
    /// copy keeps the source's age, so copying never restarts the day it is kept for.
    Copy {
        path: String,
        text: String,
        edit: Value,
    },
    /// The same edit when the drafts kept are at the cap and no copy fits: the source stays.
    Uncopied(Value),
    Nothing,
}

/// What `claim_draft` does: the draft to hand over, and the files the age sweep removes.
#[derive(Clone)]
pub struct Plan {
    pub claim: Claim,
    pub expired: Vec<String>,
}

/// What the kept drafts say to an editor that asks for one. A draft whose document no longer exists
/// is not offered; it is not removed either, it ages out like any other.
pub fn choose(
    docs: &[model::Doc],
    editor: &str,
    texts: &[String],
    now_ms: u64,
) -> Result<Plan, AppError> {
    plain("editor", editor)?;
    let mut expired = Vec::new();
    let mut live = 0;
    let mut offered = Vec::new();
    for text in texts {
        let kept = Kept::parse(text)?;
        if kept.expired(now_ms) {
            expired.push(kept.path()?);
            continue;
        }
        live += 1;
        if docs.iter().any(|doc| doc.id == kept.id) {
            offered.push(kept);
        }
    }
    let latest = |own: bool| {
        offered
            .iter()
            .filter(|kept| (kept.editor == editor) == own)
            .max_by_key(|kept| kept.saved_at_ms)
    };
    let claim = if let Some(own) = latest(true) {
        Claim::Keep(own.edit())
    } else if let Some(other) = latest(false) {
        if live < MAX_DRAFTS {
            let mut copy = other.value.clone();
            copy["editor"] = json!(editor);
            Claim::Copy {
                path: path_of(&other.id, editor)?,
                text: copy.to_string(),
                edit: other.edit(),
            }
        } else {
            Claim::Uncopied(other.edit())
        }
    } else {
        Claim::Nothing
    };
    Ok(Plan { claim, expired })
}

pub async fn save(
    docs: &[model::Doc],
    editor: &str,
    id: &str,
    base_revision: u64,
    edit: &Value,
) -> Result<Value, AppError> {
    let now = store::now_ms();
    let text = parse_new(docs, editor, id, base_revision, edit, now)?;
    let path = path_of(id, editor)?;
    let mut kept = store::private_files(FOLDER)?;
    if kept.len() >= MAX_DRAFTS && !kept.contains(&path) {
        sweep(now)?;
        kept = store::private_files(FOLDER)?;
    }
    if kept.len() >= MAX_DRAFTS && !kept.contains(&path) {
        return Err(error(
            "INVALID_STATE",
            "too many unsaved drafts are kept; save or discard one first",
        ));
    }
    store::replace(&path, text.into_bytes()).await?;
    Ok(json!({"saved": true}))
}

/// The age sweep: removes the drafts older than a day. It reads and removes without waiting in
/// between, so a draft rewritten meanwhile is never removed as the old one.
fn sweep(now_ms: u64) -> Result<(), AppError> {
    for text in store::read_private_folder(FOLDER)? {
        let Ok(kept) = Kept::parse(&text) else {
            continue;
        };
        if kept.expired(now_ms) {
            store::remove_private(&kept.path()?)?;
        }
    }
    Ok(())
}

/// The draft the editor asks for when its window starts; see `choose`. The read, the plan and the
/// sweep's removals run without waiting in between; the copy is the one write, and it is the
/// editor's own file, which no other editor writes.
pub async fn claim(docs: &[model::Doc], editor: &str) -> Result<Value, AppError> {
    let texts = store::read_private_folder(FOLDER)?;
    let plan = choose(docs, editor, &texts, store::now_ms())?;
    for path in &plan.expired {
        store::remove_private(path)?;
    }
    let draft = match plan.claim {
        Claim::Keep(edit) | Claim::Uncopied(edit) => Some(edit),
        Claim::Copy { path, text, edit } => {
            store::replace(&path, text.into_bytes()).await?;
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
    fn a_draft_of_a_missing_document_is_refused_and_one_whose_document_was_deleted_is_not_offered_or_removed(
    ) {
        let docs = docs();
        assert!(parse_new(&docs, "w1", "missing", 1, &edit("c"), 5).is_err());
        let text = kept("w1", "d1", "c", 5);
        let plan = choose(&[], "w2", &[text], 9).unwrap();
        assert!(matches!(plan.claim, Claim::Nothing));
        assert!(
            plan.expired.is_empty(),
            "only age removes another editor's draft"
        );
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
            let plan = choose(&docs(), editor, &texts, 9).unwrap();
            let Claim::Keep(draft) = plan.claim else {
                panic!("{editor} has a draft of its own");
            };
            assert_eq!(draft["content"], content);
            assert!(plan.expired.is_empty());
        }
    }

    #[test]
    fn a_new_editor_is_handed_a_copy_of_the_latest_draft_of_another_editor_that_keeps_its_age() {
        let texts = vec![
            kept("w1", "d1", "earlier", 5),
            kept("w2", "d2", "later", 9),
            kept("w3", "d1", "earliest", 1),
        ];
        let plan = choose(&docs(), "w4", &texts, 20).unwrap();
        let Claim::Copy { path, text, edit } = plan.claim else {
            panic!("a copy is handed over");
        };
        assert_eq!(
            (edit["content"].as_str(), edit["id"].as_str()),
            (Some("later"), Some("d2"))
        );
        assert_eq!(path, "drafts/d2.w4.json");
        let copy = Kept::parse(&text).unwrap();
        assert_eq!((copy.editor.as_str(), copy.saved_at_ms), ("w4", 9));
        assert_eq!(copy.path().unwrap(), path);
        assert!(plan.expired.is_empty());
    }

    #[test]
    fn an_editor_with_a_draft_takes_nothing_from_another_and_one_with_nothing_to_find_gets_nothing()
    {
        let texts = vec![
            kept("w1", "d1", "mine", 1),
            kept("w2", "d2", "newer, not mine", 9),
        ];
        let plan = choose(&docs(), "w1", &texts, 20).unwrap();
        let Claim::Keep(draft) = plan.claim else {
            panic!("its own draft is kept");
        };
        assert_eq!(draft["content"], "mine");
        let plan = choose(&docs(), "w9", &[], 20).unwrap();
        assert!(matches!(plan.claim, Claim::Nothing));
    }

    #[test]
    fn a_draft_older_than_a_day_is_swept_and_never_offered_and_one_a_day_old_is_not() {
        let day = MAX_AGE_MS;
        let texts = vec![
            kept("w1", "d1", "stale", 5),
            kept("w2", "d2", "a day old", 6),
        ];
        let plan = choose(&docs(), "w3", &texts, 6 + day).unwrap();
        assert_eq!(plan.expired, ["drafts/d1.w1.json"]);
        let Claim::Copy { edit, .. } = plan.claim else {
            panic!("the draft that is not stale is offered");
        };
        assert_eq!(edit["content"], "a day old");
        let plan = choose(&docs(), "w3", &texts, 7 + day).unwrap();
        assert_eq!(plan.expired.len(), 2);
        assert!(matches!(plan.claim, Claim::Nothing));
    }

    #[test]
    fn at_the_cap_the_draft_is_handed_over_without_a_copy_and_the_source_stays() {
        let docs = docs();
        let mut texts: Vec<String> = (0..MAX_DRAFTS)
            .map(|n| kept(&format!("w{n}"), "d1", &format!("edit {n}"), 10 + n as u64))
            .collect();
        let plan = choose(&docs, "late", &texts, 100).unwrap();
        let Claim::Uncopied(edit) = plan.claim else {
            panic!("nothing fits beside {MAX_DRAFTS} drafts");
        };
        assert_eq!(edit["content"], format!("edit {}", MAX_DRAFTS - 1));
        // Room that opens (one draft aged out) lets the copy be written.
        texts[0] = kept("w0", "d1", "edit 0", 1);
        let plan = choose(&docs, "late", &texts, 1 + MAX_AGE_MS + 1).unwrap();
        assert_eq!(plan.expired, ["drafts/d1.w0.json"]);
        assert!(matches!(plan.claim, Claim::Copy { .. }));
    }

    /// The files of the App's drafts folder, as the real `claim`, `save` and `discard` leave them.
    #[derive(Clone)]
    struct World {
        files: std::collections::BTreeMap<String, String>,
        now: u64,
        writes: u32,
        /// What each editor last wrote or was handed, with the time its file was written: the edit
        /// it would lose if its file changed under it.
        owns: std::collections::BTreeMap<&'static str, (String, u64)>,
        /// A claim read and planned, waiting for its one write.
        claiming: std::collections::BTreeMap<&'static str, (Plan, u64)>,
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Save(&'static str),
        Discard(&'static str),
        Begin(&'static str),
        Finish(&'static str),
        /// More than a day passes.
        Tick,
    }

    const EDITORS: [&str; 2] = ["a", "b"];

    fn steps() -> Vec<Step> {
        let mut steps = vec![Step::Tick];
        for editor in EDITORS {
            steps.extend([
                Step::Save(editor),
                Step::Discard(editor),
                Step::Begin(editor),
                Step::Finish(editor),
            ]);
        }
        steps
    }

    impl World {
        fn new() -> Self {
            Self {
                files: Default::default(),
                now: 1_000,
                writes: 0,
                owns: Default::default(),
                claiming: Default::default(),
            }
        }

        fn texts(&self) -> Vec<String> {
            self.files.values().cloned().collect()
        }

        /// One editor's calls run one at a time, so it has nothing else to do while its claim waits.
        fn allowed(&self, step: Step) -> bool {
            match step {
                Step::Save(e) | Step::Discard(e) | Step::Begin(e) => !self.claiming.contains_key(e),
                Step::Finish(e) => self.claiming.contains_key(e),
                Step::Tick => true,
            }
        }

        /// Applies the step with the real `parse_new`, `choose` and the removals `claim` makes, and
        /// returns every path the step changed.
        fn step(&mut self, step: Step) -> Vec<String> {
            let before = self.files.clone();
            match step {
                Step::Tick => self.now += MAX_AGE_MS + 1,
                Step::Save(editor) => {
                    self.writes += 1;
                    let content = format!("{editor}{}", self.writes);
                    let text =
                        parse_new(&docs(), editor, "d1", 1, &edit(&content), self.now).unwrap();
                    self.files.insert(path_of("d1", editor).unwrap(), text);
                    self.owns.insert(editor, (content, self.now));
                }
                Step::Discard(editor) => {
                    self.files.remove(&path_of("d1", editor).unwrap());
                    self.owns.remove(editor);
                }
                Step::Begin(editor) => {
                    let plan = choose(&docs(), editor, &self.texts(), self.now).unwrap();
                    for path in &plan.expired {
                        self.files.remove(path);
                    }
                    self.claiming.insert(editor, (plan, self.now));
                }
                Step::Finish(editor) => {
                    let (plan, _) = self.claiming.remove(editor).unwrap();
                    match plan.claim {
                        Claim::Copy { path, text, edit } => {
                            let at = Kept::parse(&text).unwrap().saved_at_ms;
                            self.files.insert(path, text);
                            let content = edit["content"].as_str().unwrap().to_owned();
                            self.owns.insert(editor, (content, at));
                        }
                        Claim::Keep(_) | Claim::Uncopied(_) | Claim::Nothing => {}
                    }
                }
            }
            let mut changed: Vec<String> = before
                .iter()
                .filter(|(path, text)| self.files.get(*path) != Some(text))
                .map(|(path, _)| path.clone())
                .collect();
            changed.extend(
                self.files
                    .keys()
                    .filter(|p| !before.contains_key(*p))
                    .cloned(),
            );
            changed
        }

        /// What no sequence of steps may break.
        fn check(&self, step: Step, changed: &[String], before: &World) {
            let own = |editor: &str| path_of("d1", editor).unwrap();
            // A step writes or removes only its own editor's file; the age sweep, run by a claim,
            // is the one removal of anyone else's, and only of a draft older than a day.
            for path in changed {
                let owner = match step {
                    Step::Save(e) | Step::Discard(e) | Step::Finish(e) => Some(own(e)),
                    Step::Begin(_) | Step::Tick => None,
                };
                if owner.as_ref() == Some(path) {
                    continue;
                }
                let swept = matches!(step, Step::Begin(_))
                    && !self.files.contains_key(path)
                    && before
                        .files
                        .get(path)
                        .is_some_and(|text| Kept::parse(text).unwrap().expired(before.now));
                assert!(
                    swept,
                    "{step:?} changed {path}, which is not its editor's own"
                );
            }
            // The edit an editor kept or was handed stays in its own file until it discards it
            // or the file is a day old.
            for (editor, (content, at)) in &self.owns {
                if self.now.saturating_sub(*at) > MAX_AGE_MS {
                    continue;
                }
                let kept = self
                    .files
                    .get(&own(editor))
                    .map(|t| Kept::parse(t).unwrap());
                assert_eq!(
                    kept.as_ref().map(|k| k.value["content"].as_str().unwrap()),
                    Some(content.as_str()),
                    "after {step:?} editor {editor} lost its edit"
                );
            }
        }

        /// The edit a claim hands over is one some draft held when it read them.
        fn check_handed(&self, editor: &str, before: &World) {
            let (plan, _) = &self.claiming[editor];
            let edit = match &plan.claim {
                Claim::Keep(edit) | Claim::Uncopied(edit) | Claim::Copy { edit, .. } => edit,
                Claim::Nothing => return,
            };
            assert!(
                before
                    .files
                    .values()
                    .any(|text| Kept::parse(text).unwrap().value["content"] == edit["content"]),
                "claim by {editor} handed an edit no draft held"
            );
        }
    }

    fn explore(world: &World, depth: usize, steps: &[Step], visited: &mut u64) {
        if depth == 0 {
            return;
        }
        for &step in steps {
            if !world.allowed(step) {
                continue;
            }
            let mut next = world.clone();
            let changed = next.step(step);
            next.check(step, &changed, world);
            if let Step::Begin(editor) = step {
                next.check_handed(editor, world);
            }
            *visited += 1;
            explore(&next, depth - 1, steps, visited);
        }
    }

    /// Two editors that save, discard, claim, with the claim's read and its write apart, a day
    /// passing between any two steps: every order of up to `DEPTH` steps keeps every editor's edit in
    /// its own file and changes no file but its own editor's (or an expired one, swept).
    #[test]
    fn no_order_of_saves_discards_claims_and_days_loses_an_edit_or_touches_another_editors_draft() {
        const DEPTH: usize = 6;
        let mut visited = 0;
        explore(&World::new(), DEPTH, &steps(), &mut visited);
        assert!(visited > 50_000, "the search covered {visited} steps");
    }

    /// The reviewer's first repro, in order: the live editor's only draft, another editor opens
    /// and discards, and the live editor's window is replaced.
    #[test]
    fn a_live_editors_only_draft_survives_another_editor_claiming_and_discarding() {
        let mut world = World::new();
        for step in [
            Step::Save("a"),
            Step::Begin("b"),
            Step::Finish("b"),
            Step::Discard("b"),
        ] {
            world.step(step);
        }
        let a = world.files[&path_of("d1", "a").unwrap()].clone();
        assert_eq!(Kept::parse(&a).unwrap().value["content"], "a1");
        // Its successor is handed the edit.
        world.step(Step::Begin("b"));
        assert!(matches!(world.claiming["b"].0.claim, Claim::Copy { .. }));
    }
}
