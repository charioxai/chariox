//! The unsaved scene of each open window. The window's page keeps nothing across a replacement (an
//! App update, a reconnect, a reload), so it hands the scene it is about to save to the App and asks
//! for the kept ones when it starts. A window has no identity that outlives its replacement, so each
//! window keeps its drafts under an id of its own choosing, and only that window removes its draft
//! (`discard_draft`): one window saving never drops what another window has not saved. A recovering
//! window lists the drafts and reads them one at a time, so no reply nears the catalog's call limit,
//! and merges them into its canvas. It leaves them where they are until its own draft or the saved
//! scene holds what it merged; then it retires exactly the revision it read (`retire_draft`), so a
//! window that is still open and has written since keeps its newer draft, and replacements never pile
//! drafts up against the cap. The other removal that is not the window's own is age: a draft that
//! cannot be read, or that is older than the hour a deletion is kept, is dropped when the folder is next
//! read or written. At the cap a new window's draft is refused, never made room for by removing another.
//!
//! A recovered draft is merged into the saved scene by element version, as any other saved scene
//! is, so it needs no base version and never overwrites what was saved meanwhile. Merging the same draft
//! again changes nothing.

use crate::{actions, file_io, scene};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};
use std::{fs, io::ErrorKind};

const FOLDER: &str = "drafts";
const MAX_ID: usize = 64;
/// The most drafts kept at once, so a reply with all of them stays small. A window with a new id is
/// refused while this many are kept; they age out within the hour.
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

/// A kept draft: the window it belongs to and its revision (the time it was written, which rises with
/// every write of that window).
struct Kept {
    id: String,
    revision: u64,
}

/// The drafts a window can recover, oldest first, and the ids whose files are of no use any more: one
/// that cannot be read, or one kept longer ago than the deletions are, because a deletion that an
/// older draft does not know of may be forgotten by then and the draft would draw it again.
fn select(files: Vec<(String, String)>, now_ms: u64) -> (Vec<Kept>, Vec<String>) {
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
        kept.into_iter()
            .map(|(revision, id, _)| Kept { id, revision })
            .collect(),
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

/// What a save finds in the folder: the files to drop for their age, and whether the window's id fits.
/// An id that is kept already always fits, since the save replaces its file.
fn admit(files: Vec<(String, String)>, id: &str, now_ms: u64) -> (Vec<String>, bool) {
    let (kept, useless) = select(files, now_ms);
    let others = kept.iter().filter(|draft| draft.id != id).count();
    (useless, others < MAX_DRAFTS)
}

/// `save_draft`: keep the scene this window is about to save, under its own id.
pub async fn save(input: &Value) -> Result<Value, AppError> {
    let id = input["id"]
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", "id is required"))?;
    let path = path_of(id)?;
    let checked = scene::parse(&input["scene"].to_string())?;
    let now = now_ms();
    let files = files()?;
    // Two writes of one window never share a revision, so a retirement names exactly one of them.
    let revision = files
        .iter()
        .find(|(other, _)| other == id)
        .and_then(|(_, text)| decode(text))
        .map_or(now, |(previous, _)| now.max(previous + 1));
    let (useless, fits) = admit(files, id, now);
    for id in useless {
        remove(&id)?;
    }
    if !fits {
        return Err(error(
            "INVALID_STATE",
            format!("{MAX_DRAFTS} other windows already keep an unsaved diagram; save or close one, or wait until it is an hour old"),
        ));
    }
    // The broker's atomic replace does not create parents.
    fs::create_dir_all(format!("data/{FOLDER}")).map_err(|_| {
        error(
            "DRAFT_FAILED",
            "cannot create the folder of the kept drafts",
        )
    })?;
    file_io::replace(path, encode(&checked, revision).into_bytes()).await?;
    Ok(json!({}))
}

/// `list_drafts`: the kept drafts' ids and revisions, oldest first. A scene is read one at a time with
/// `read_draft`, because the scenes of several windows together can exceed the catalog's call limit.
pub fn list() -> Result<Value, AppError> {
    let (kept, useless) = select(files()?, now_ms());
    for id in useless {
        remove(&id)?;
    }
    Ok(json!({"drafts": kept.into_iter()
        .map(|draft| json!({"id": draft.id, "revision": draft.revision}))
        .collect::<Vec<_>>()}))
}

/// The kept draft of `id`, when it is still kept and readable.
fn kept_of(id: &str) -> Result<Option<(u64, Value)>, AppError> {
    match fs::read_to_string(format!("data/{}", path_of(id)?)) {
        Ok(text) => Ok(decode(&text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error("DRAFT_FAILED", "cannot read the kept draft")),
    }
}

/// `read_draft`: the scene of one kept draft. A draft that its window discarded or that aged out since
/// the list has no scene to hand over.
pub fn read(input: &Value) -> Result<Value, AppError> {
    let id = input["id"]
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", "id is required"))?;
    Ok(match kept_of(id)? {
        Some((revision, scene)) => json!({"id": id, "revision": revision, "scene": scene}),
        None => json!({"id": id, "revision": null, "scene": null}),
    })
}

/// `retire_draft`: forget the draft of this id when it is still at the revision the caller recovered
/// and now holds the content elsewhere. A window that is still open and has written since keeps its
/// newer draft.
pub fn retire(input: &Value) -> Result<Value, AppError> {
    let id = input["id"]
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", "id is required"))?;
    let revision = input["revision"]
        .as_u64()
        .ok_or_else(|| error("INVALID_ARGUMENT", "revision is required"))?;
    let current = kept_of(id)?.is_some_and(|(kept, _)| kept == revision);
    if current {
        remove(id)?;
    }
    Ok(json!({"retired": current}))
}

/// `discard_draft`: forget the draft of this id and no other. Only the window of that id sends it.
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
        let ids: Vec<&str> = kept.iter().map(|draft| draft.id.as_str()).collect();
        assert_eq!(ids, ["old", "new"]);
        assert_eq!(kept[1].revision, now - 5);
        assert_eq!(useless, ["expired", "torn", "not-a-scene"]);
    }

    #[test]
    fn a_new_window_is_refused_at_the_cap_and_a_kept_one_is_not_and_nothing_is_dropped_to_make_room(
    ) {
        let now = actions::TOMBSTONE_MS * 10;
        let files: Vec<(String, String)> = (0..MAX_DRAFTS)
            .map(|n| (format!("w{n}"), encode(&scene_of(&["x"]), now - n as u64)))
            .collect();
        let (useless, fits) = admit(files.clone(), "new", now);
        assert!(!fits && useless.is_empty());
        let (useless, fits) = admit(files.clone(), "w0", now);
        assert!(
            fits && useless.is_empty(),
            "a window that keeps a draft rewrites its own"
        );
        // Room opens only when a draft ages out, and that one is the only file dropped.
        let later = now + actions::TOMBSTONE_MS - (MAX_DRAFTS as u64 - 1);
        let (useless, fits) = admit(files, "new", later);
        assert_eq!(useless, ["w3"]);
        assert!(fits);
    }

    /// Windows `a`, `b` and `c` keep, read and discard drafts, and an hour passes between any two
    /// steps. Every order of up to `DEPTH` steps writes or removes only the file of the window that
    /// sent the step, except that an hour-old or unreadable file is swept by whatever reads or writes
    /// the folder next; and a window's kept scene stays in its file until it discards it.
    #[test]
    fn no_order_of_saves_reads_discards_and_hours_removes_another_windows_draft() {
        use std::collections::BTreeMap;
        #[derive(Clone, Copy, Debug)]
        enum Step {
            Save(&'static str),
            Discard(&'static str),
            Read,
            Hour,
        }
        #[derive(Clone)]
        struct World {
            files: BTreeMap<String, String>,
            now: u64,
            writes: u64,
            /// The scene each window last kept, with when.
            kept: BTreeMap<&'static str, (u64, u64)>,
        }
        const DEPTH: usize = 6;
        let windows = ["a", "b", "c"];
        let mut steps = vec![Step::Read, Step::Hour];
        for window in windows {
            steps.extend([Step::Save(window), Step::Discard(window)]);
        }
        fn run(world: &World, depth: usize, steps: &[Step], visited: &mut u64) {
            if depth == 0 {
                return;
            }
            for &step in steps {
                let mut next = world.clone();
                let before = next.files.clone();
                match step {
                    Step::Hour => next.now += actions::TOMBSTONE_MS,
                    Step::Read => {
                        let files = next.files.iter().map(|(i, t)| (i.clone(), t.clone()));
                        for id in select(files.collect(), next.now).1 {
                            next.files.remove(&id);
                        }
                    }
                    Step::Save(window) => {
                        let files = next.files.iter().map(|(i, t)| (i.clone(), t.clone()));
                        let (useless, fits) = admit(files.collect(), window, next.now);
                        for id in useless {
                            next.files.remove(&id);
                        }
                        if fits {
                            next.writes += 1;
                            let scene = scene_of(&[&format!("e{}", next.writes)]);
                            next.files
                                .insert(window.to_owned(), encode(&scene, next.now));
                            next.kept.insert(window, (next.writes, next.now));
                        }
                    }
                    Step::Discard(window) => {
                        next.files.remove(window);
                        next.kept.remove(window);
                    }
                }
                for (id, text) in &before {
                    if next.files.get(id) == Some(text) {
                        continue;
                    }
                    let own = matches!(step, Step::Save(w) | Step::Discard(w) if w == id);
                    let swept = !next.files.contains_key(id)
                        && decode(text).is_none_or(|(at, _)| {
                            next.now.saturating_sub(at) >= actions::TOMBSTONE_MS
                        });
                    assert!(
                        own || swept,
                        "{step:?} changed {id}, which is not its window's own"
                    );
                }
                for (id, text) in &next.files {
                    assert!(
                        before.get(id) == Some(text) || matches!(step, Step::Save(w) if w == id)
                    );
                }
                for (window, (n, at)) in &next.kept {
                    if next.now.saturating_sub(*at) >= actions::TOMBSTONE_MS {
                        continue;
                    }
                    let text = next
                        .files
                        .get(*window)
                        .unwrap_or_else(|| panic!("after {step:?} window {window} lost its draft"));
                    let (_, scene) = decode(text).unwrap();
                    assert_eq!(scene, scene_of(&[&format!("e{n}")]));
                }
                *visited += 1;
                run(&next, depth - 1, steps, visited);
            }
        }
        let start = World {
            files: BTreeMap::new(),
            now: actions::TOMBSTONE_MS * 3,
            writes: 0,
            kept: BTreeMap::new(),
        };
        let mut visited = 0;
        run(&start, DEPTH, &steps, &mut visited);
        assert!(visited > 100_000, "the search covered {visited} steps");
    }
}
