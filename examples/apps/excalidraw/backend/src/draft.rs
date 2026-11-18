//! The unsaved scene of each open window. The window's page keeps nothing across a replacement (an
//! App update, a reconnect, a reload), so it hands the scene it is about to save to the App and asks
//! for the kept ones when it starts. A window has no identity that outlives its replacement, so each
//! window keeps its drafts under an id of its own choosing and discards only that id's draft: one
//! window saving never drops what another window has not saved.
//!
//! A recovered draft is merged into the saved scene by element version, as any other saved scene
//! is, so it needs no base version and never overwrites what was saved meanwhile.

use crate::{actions, file_io, scene};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};
use std::{fs, io::ErrorKind};

const FOLDER: &str = "drafts";
const MAX_ID: usize = 64;
/// The most drafts kept at once: the oldest goes when a window with a new id keeps one more, so the
/// drafts of windows that are gone cannot pile up, and a reply with all of them stays small.
const MAX_DRAFTS: usize = 4;

/// Where the draft of a window is kept. The id names a file, so it must look like one id.
fn path_of(id: &str) -> Result<String, AppError> {
    let plain = !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !plain {
        return Err(error("INVALID_ARGUMENT", "id is not a window id"));
    }
    Ok(format!("{FOLDER}/{id}.json"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The text of a draft file: the scene and when it was kept.
fn encode(scene: &Value, saved_at_ms: u64) -> String {
    json!({"saved_at_ms": saved_at_ms, "scene": scene}).to_string()
}

/// A draft file's scene and time, or none when it cannot be read as a draft.
fn decode(text: &str) -> Option<(u64, Value)> {
    let mut draft: Value = serde_json::from_str(text).ok()?;
    let saved_at = draft["saved_at_ms"].as_u64()?;
    let scene = draft["scene"].take();
    scene::parse(&scene.to_string()).ok()?;
    Some((saved_at, scene))
}

/// The drafts a window is handed, oldest first, and the ids whose files are of no use any more: one
/// that cannot be read, or one kept longer ago than the deletions are, because a deletion that an
/// older draft does not know of may be forgotten by then and the draft would draw it again.
fn select(files: Vec<(String, String)>, now_ms: u64) -> (Vec<(String, Value)>, Vec<String>) {
    let mut kept = Vec::new();
    let mut useless = Vec::new();
    for (id, text) in files {
        match decode(&text) {
            Some((saved_at, scene)) if now_ms.saturating_sub(saved_at) < actions::TOMBSTONE_MS => {
                kept.push((saved_at, id, scene))
            }
            _ => useless.push(id),
        }
    }
    kept.sort_by_key(|(saved_at, _, _)| *saved_at);
    (
        kept.into_iter().map(|(_, id, scene)| (id, scene)).collect(),
        useless,
    )
}

/// The files of the draft folder as (id, text); none while nothing was kept.
fn files() -> Result<Vec<(String, String)>, AppError> {
    let entries = match fs::read_dir(format!("data/{FOLDER}")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(error("DRAFT_FAILED", "cannot list the kept drafts")),
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        // A file removed after the listing was discarded by its window: nothing to hand over.
        if let Ok(text) = fs::read_to_string(entry.path()) {
            found.push((id.to_owned(), text));
        }
    }
    Ok(found)
}

fn remove(id: &str) -> Result<(), AppError> {
    match fs::remove_file(format!("data/{}", path_of(id)?)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error("DRAFT_FAILED", "cannot discard the kept draft")),
    }
}

/// `save_draft`: keep the scene this window is about to save, under its own id.
pub async fn save(input: &Value) -> Result<Value, AppError> {
    let id = input["id"]
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", "id is required"))?;
    let path = path_of(id)?;
    let checked = scene::parse(&input["scene"].to_string())?;
    let now = now_ms();
    let (kept, useless) = select(files()?, now);
    for id in useless {
        remove(&id)?;
    }
    // Room for this one: a window with an id the App has not seen pushes out the oldest draft.
    let others: Vec<&String> = kept
        .iter()
        .map(|(other, _)| other)
        .filter(|other| other.as_str() != id)
        .collect();
    let overflow = (others.len() + 1).saturating_sub(MAX_DRAFTS);
    for other in others.into_iter().take(overflow) {
        remove(other)?;
    }
    // The broker's atomic replace does not create parents.
    fs::create_dir_all(format!("data/{FOLDER}")).map_err(|_| {
        error(
            "DRAFT_FAILED",
            "cannot create the folder of the kept drafts",
        )
    })?;
    file_io::replace(path, encode(&checked, now).into_bytes()).await?;
    Ok(json!({}))
}

/// `read_drafts`: the kept drafts, oldest first.
pub fn read() -> Result<Value, AppError> {
    let (kept, useless) = select(files()?, now_ms());
    for id in useless {
        remove(&id)?;
    }
    Ok(
        json!({"drafts": kept.into_iter().map(|(id, scene)| json!({"id": id, "scene": scene})).collect::<Vec<_>>()}),
    )
}

/// `discard_draft`: forget the draft of this id and no other.
pub fn discard(input: &Value) -> Result<Value, AppError> {
    remove(
        input["id"]
            .as_str()
            .ok_or_else(|| error("INVALID_ARGUMENT", "id is required"))?,
    )?;
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_of(ids: &[&str]) -> Value {
        let mut scene = scene::empty();
        scene["elements"] = json!(ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>());
        scene
    }

    #[test]
    fn each_window_has_its_own_draft_file_and_an_id_cannot_name_another_path() {
        assert_eq!(path_of("w-1_a").unwrap(), "drafts/w-1_a.json");
        for id in ["", "../scene", "a/b", "a.b", "a b", &"x".repeat(MAX_ID + 1)] {
            assert!(path_of(id).is_err(), "{id:?}");
        }
    }

    #[test]
    fn the_drafts_handed_over_are_the_recent_readable_ones_oldest_first() {
        let hour = actions::TOMBSTONE_MS;
        let now = 10 * hour;
        let files = vec![
            ("new".to_owned(), encode(&scene_of(&["n"]), now - 5)),
            ("old".to_owned(), encode(&scene_of(&["o"]), now - 50)),
            ("expired".to_owned(), encode(&scene_of(&["e"]), now - hour)),
            ("torn".to_owned(), "{\"saved_at".to_owned()),
            (
                "not-a-scene".to_owned(),
                json!({"saved_at_ms": now, "scene": {"type": "other"}}).to_string(),
            ),
        ];
        let (kept, useless) = select(files, now);
        let ids: Vec<&str> = kept.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["old", "new"]);
        assert_eq!(kept[1].1, scene_of(&["n"]));
        assert_eq!(useless, ["expired", "torn", "not-a-scene"]);
    }
}
