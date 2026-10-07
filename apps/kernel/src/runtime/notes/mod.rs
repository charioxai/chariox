//! MD-N1 / MP-08 / MP-11: one durable owner store below every client.
use crate::local::{
    NoteAnchor, NoteBox, NoteRecord, NoteReply, NoteSelection, NoteSummary, NoteWindow,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) mod observation;
mod validation;
pub(crate) use validation::{validate_anchor, validate_box, validate_quote, validate_window};
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct NoteStore {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}

fn failure(_: impl std::fmt::Display) -> String {
    "MD-N1: note storage unavailable".into()
}
fn identity() -> String {
    format!("note-{:032x}", rand::random::<u128>())
}
fn now() -> u64 {
    crate::session::unix_epoch_ms()
}

impl NoteStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: root.join("notes"),
            lock: Arc::default(),
        }
    }
    fn with_db<T>(
        &self,
        call: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let _lock = self.lock.lock().map_err(failure)?;
        std::fs::create_dir_all(&self.root).map_err(failure)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
                .map_err(failure)?;
            // Create privately before SQLite opens it (including its first write).
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(self.root.join("notes.sqlite3"))
                .map_err(failure)?;
        }
        let mut db = Connection::open(self.root.join("notes.sqlite3")).map_err(failure)?;
        db.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(failure)?;
        db.execute_batch("PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS notes(owner TEXT NOT NULL, id TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(owner,id));
            CREATE TABLE IF NOT EXISTS selections(owner TEXT NOT NULL, id TEXT NOT NULL, record TEXT NOT NULL, expires INTEGER NOT NULL, note_id TEXT, PRIMARY KEY(owner,id));
            CREATE TABLE IF NOT EXISTS note_audit(sequence INTEGER PRIMARY KEY, owner TEXT NOT NULL, note_id TEXT, actor TEXT NOT NULL, operation TEXT NOT NULL, at_ms INTEGER NOT NULL);").map_err(failure)?;
        call(&mut db)
    }
    pub(crate) fn selection(
        &self,
        owner: &str,
        anchor: NoteAnchor,
        box_css: Option<NoteBox>,
    ) -> Result<NoteSelection, String> {
        validate_anchor(&anchor)?;
        validate_box(box_css.as_ref())?;
        let selection = NoteSelection {
            selection_id: identity(),
            anchor,
            box_css,
        };
        self.with_db(|db| {
            db.execute("DELETE FROM selections WHERE expires < ?1", [now() as i64])
                .map_err(failure)?;
            let existing: Vec<String> = {
                let mut query = db
                    .prepare("SELECT record FROM selections WHERE owner=?1 AND note_id IS NULL")
                    .map_err(failure)?;
                let rows = query
                    .query_map([owner], |row| row.get::<_, String>(0))
                    .map_err(failure)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(failure)?
            };
            for record in existing {
                let previous: NoteSelection = serde_json::from_str(&record).map_err(failure)?;
                if !validation::same_window(&previous.anchor.window, &selection.anchor.window) {
                    continue;
                }
                if previous.anchor == selection.anchor {
                    let updated = NoteSelection {
                        selection_id: previous.selection_id,
                        ..selection
                    };
                    if previous.box_css != updated.box_css {
                        db.execute(
                            "UPDATE selections SET record=?3 WHERE owner=?1 AND id=?2",
                            params![
                                owner,
                                updated.selection_id,
                                serde_json::to_string(&updated).map_err(failure)?
                            ],
                        )
                        .map_err(failure)?;
                    }
                    return Ok(updated);
                }
                db.execute(
                    "DELETE FROM selections WHERE owner=?1 AND id=?2",
                    params![owner, previous.selection_id],
                )
                .map_err(failure)?;
            }
            let count: u32 = db
                .query_row(
                    "SELECT count(*) FROM selections WHERE owner=?1",
                    [owner],
                    |r| r.get(0),
                )
                .map_err(failure)?;
            if count >= 128 {
                return Err("MD-N1: selection limit reached".into());
            }
            db.execute(
                "INSERT INTO selections VALUES(?1,?2,?3,?4,NULL)",
                params![
                    owner,
                    selection.selection_id,
                    serde_json::to_string(&selection).map_err(failure)?,
                    (now() + 600_000) as i64
                ],
            )
            .map_err(failure)?;
            Ok(selection)
        })
    }
    pub(crate) fn create(
        &self,
        owner: &str,
        selection_id: &str,
        comment: &str,
    ) -> Result<NoteRecord, String> {
        validation::validate_comment(comment)?;
        self.with_db(|db| {
            let tx = db.transaction().map_err(failure)?;
            let selected: Option<(String, Option<String>)> = tx.query_row("SELECT record,note_id FROM selections WHERE owner=?1 AND id=?2 AND expires>=?3", params![owner,selection_id,now() as i64], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(failure)?;
            let (record, previous) = selected.ok_or("MD-N1: selection unavailable; select again")?;
            if let Some(id) = previous {
                let note = read(&tx, owner, &id)?;
                if note.comment != comment { return Err("MD-N1: selection already used with a different comment".into()); }
                return Ok(note);
            }
            let count: u32 = tx.query_row("SELECT count(*) FROM notes WHERE owner=?1", [owner], |r| r.get(0)).map_err(failure)?;
            if count >= 8192 { return Err("MD-N1: note limit reached".into()); }
            let selection: NoteSelection = serde_json::from_str(&record).map_err(failure)?;
            let stamp = now();
            let note = NoteRecord { note_id: identity(), author: owner.into(), domain: match selection.anchor.window { NoteWindow::RoomBrowser {..} | NoteWindow::Terminal {..} => "agent", _ => "user" }.into(), anchor: selection.anchor, comment: comment.into(), replies: Vec::new(), resolved: false, anchor_state: "attached".into(), box_css: selection.box_css, created_at_ms: stamp, updated_at_ms: stamp };
            write(&tx, owner, &note)?;
            audit(&tx, owner, Some(&note.note_id), owner, "create")?;
            tx.execute("UPDATE selections SET note_id=?3 WHERE owner=?1 AND id=?2", params![owner,selection_id,note.note_id]).map_err(failure)?;
            tx.commit().map_err(failure)?;
            Ok(note)
        })
    }
    pub(crate) fn read(&self, owner: &str, id: &str, actor: &str) -> Result<NoteRecord, String> {
        self.with_db(|db| {
            let note = read(db, owner, id)?;
            audit(db, owner, Some(id), actor, "read")?;
            Ok(note)
        })
    }
    pub(crate) fn list(
        &self,
        owner: &str,
        window: &NoteWindow,
        actor: &str,
    ) -> Result<Vec<NoteSummary>, String> {
        validate_window(window)?;
        self.with_db(|db| {
            let mut query = db
                .prepare("SELECT record FROM notes WHERE owner=?1 ORDER BY id")
                .map_err(failure)?;
            let rows = query
                .query_map([owner], |row| row.get::<_, String>(0))
                .map_err(failure)?;
            let mut notes = Vec::new();
            for row in rows {
                let note: NoteRecord =
                    serde_json::from_str(&row.map_err(failure)?).map_err(failure)?;
                if validation::same_window(&note.anchor.window, window) {
                    notes.push(NoteSummary {
                        note_id: note.note_id,
                        window: note.anchor.window,
                        resolved: note.resolved,
                        anchor_state: note.anchor_state,
                        updated_at_ms: note.updated_at_ms,
                    });
                }
            }
            audit(db, owner, None, actor, "list")?;
            Ok(notes)
        })
    }
    pub(crate) fn reply(
        &self,
        owner: &str,
        id: &str,
        actor: &str,
        comment: &str,
    ) -> Result<NoteRecord, String> {
        validation::validate_comment(comment)?;
        self.update(owner, id, actor, "reply", |note| {
            if note.replies.len() >= 64 {
                return Err("MD-N1: reply limit reached".into());
            }
            if note.resolved {
                return Err("MD-N1: note is resolved".into());
            }
            note.replies.push(NoteReply {
                author: actor.into(),
                comment: comment.into(),
                created_at_ms: now(),
            });
            Ok(())
        })
    }
    pub(crate) fn resolve(&self, owner: &str, id: &str, actor: &str) -> Result<NoteRecord, String> {
        self.update(owner, id, actor, "resolve", |note| {
            note.resolved = true;
            Ok(())
        })
    }
    pub(crate) fn reanchor(
        &self,
        owner: &str,
        id: &str,
        actor: &str,
        state: &str,
        box_css: Option<NoteBox>,
        document_id: Option<String>,
    ) -> Result<NoteRecord, String> {
        if !["attached", "missing", "ambiguous", "unavailable"].contains(&state) {
            return Err("MD-N2: invalid anchor state".into());
        }
        validate_box(box_css.as_ref())?;
        self.update(owner, id, actor, "reanchor", |note| {
            note.anchor_state = state.into();
            note.box_css = box_css;
            note.anchor.document_id = document_id;
            Ok(())
        })
    }
    fn update(
        &self,
        owner: &str,
        id: &str,
        actor: &str,
        operation: &str,
        change: impl FnOnce(&mut NoteRecord) -> Result<(), String>,
    ) -> Result<NoteRecord, String> {
        self.with_db(|db| {
            let tx = db.transaction().map_err(failure)?;
            let mut note = read(&tx, owner, id)?;
            let previous = note.clone();
            change(&mut note)?;
            if note != previous {
                note.updated_at_ms = now();
                write(&tx, owner, &note)?;
                audit(&tx, owner, Some(id), actor, operation)?;
            }
            tx.commit().map_err(failure)?;
            Ok(note)
        })
    }
    pub(crate) fn ask(
        &self,
        owner: &str,
        id: &str,
        actor: &str,
    ) -> Result<crate::local::NoteResult, String> {
        use base64::Engine;
        self.with_db(|db| {
            let note = read(db, owner, id)?;
            let text = format!(
                "[Chariox note {}]\nSelected text:\n{}\nComment:\n{}\n[/Chariox note]",
                note.note_id, note.anchor.quote.exact, note.comment
            );
            let attachment = crate::session::PromptAttachment::new(
                format!(
                    "chariox-terminal://prompt-attachment/{}/note.txt",
                    note.note_id
                ),
                "text/plain",
                Some("Chariox note.txt".into()),
            )
            .with_contents_base64(
                base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
            );
            audit(db, owner, Some(id), actor, "ask_draft")?;
            Ok(crate::local::NoteResult::PromptDraft {
                note_id: id.into(),
                text,
                attachment,
            })
        })
    }
}
fn read(db: &Connection, owner: &str, id: &str) -> Result<NoteRecord, String> {
    let record: Option<String> = db
        .query_row(
            "SELECT record FROM notes WHERE owner=?1 AND id=?2",
            params![owner, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(failure)?;
    serde_json::from_str(&record.ok_or("MD-N1: note unavailable")?).map_err(failure)
}
fn write(db: &Connection, owner: &str, note: &NoteRecord) -> Result<(), String> {
    db.execute("INSERT INTO notes VALUES(?1,?2,?3) ON CONFLICT(owner,id) DO UPDATE SET record=excluded.record",params![owner,note.note_id,serde_json::to_string(note).map_err(failure)?]).map_err(failure)?;
    Ok(())
}
fn audit(
    db: &Connection,
    owner: &str,
    id: Option<&str>,
    actor: &str,
    operation: &str,
) -> Result<(), String> {
    db.execute(
        "INSERT INTO note_audit(owner,note_id,actor,operation,at_ms) VALUES(?1,?2,?3,?4,?5)",
        params![owner, id, actor, operation, now() as i64],
    )
    .map_err(failure)?;
    Ok(())
}
