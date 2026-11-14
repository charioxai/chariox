//! The saved scene: the one state record that the window and the agent's actions both write, each
//! with the version it read, so neither overwrites what the other saved. Every write adds its
//! change to the change feed of the App's view.

use crate::{actions, scene};
use chariox_app_sdk::{app_types::AppError, error, state, view};
use serde_json::{json, Value};

const KEY: &str = "scene";

pub struct Saved {
    pub scene: Value,
    /// The version of the record; none before the first save.
    pub version: Option<u64>,
}

pub async fn load() -> Result<Saved, AppError> {
    Ok(match state::get(KEY.into()).await? {
        Some(record) => Saved {
            scene: scene::parse(&record.value)?,
            version: Some(record.version),
        },
        None => Saved {
            scene: scene::empty(),
            version: None,
        },
    })
}

/// Save `scene` over the version that was read; the new version is returned. A save that lost
/// the race is refused, so the caller reads again and merges.
pub async fn save(scene: &Value, expected: Option<u64>, change: Value) -> Result<u64, AppError> {
    let write = view::transact_with_change(
        state::Transaction {
            schema_version: 0,
            checks: vec![state::Check {
                key: KEY.into(),
                version: expected,
            }],
            writes: vec![state::Write::Put((KEY.into(), scene.to_string()))],
            occurrences: vec![],
            wakes: vec![],
        },
        change,
    )
    .await;
    let current = state::get(KEY.into()).await?.map(|record| record.version);
    match write {
        Ok(_) => current.ok_or_else(|| error("INVALID_STATE", "the saved diagram is missing")),
        Err(_) if current != expected => Err(error(
            "SCENE_CONFLICT",
            "the diagram was changed by another window or an agent; load it again and retry",
        )),
        Err(failure) => Err(failure),
    }
}

/// What the view reports as its state: the saved diagram's summary.
pub async fn view_state() -> Result<Value, AppError> {
    Ok(actions::summary(&load().await?.scene, 0, 50))
}

/// The diagram without the elements that are deleted, as a file or a person would have it.
pub fn live_scene(scene: &Value) -> Value {
    let mut live = scene.clone();
    live["elements"] = json!(scene["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| crate::element::live(e))
        .collect::<Vec<_>>());
    live
}
