//! The diagram actions an agent calls: editing the saved scene, and saving, opening and
//! exporting it as a file. The window's own saves go through `save_scene` in `lib.rs`.

use crate::stamp::Stamp;
use crate::{actions, file_io, saved, scene};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

/// The most element IDs a change in the feed names, so that a change stays small.
const CHANGE_IDS: usize = 32;

fn now() -> Result<std::time::Duration, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| error("INVALID_STATE", "cannot read the clock"))
}

fn stamp() -> Result<Stamp, AppError> {
    let now = now()?;
    Ok(Stamp::new(now.as_nanos() as u64, now.as_millis() as u64))
}

/// The element IDs a request names, for the change feed.
fn named(input: &Value) -> Value {
    let mut ids: Vec<&str> = Vec::new();
    for list in [&input["ids"], &input["elements"]] {
        for entry in list.as_array().into_iter().flatten() {
            ids.extend(entry.as_str().or_else(|| entry["id"].as_str()));
        }
    }
    ids.extend(["from", "to"].iter().filter_map(|key| input[*key].as_str()));
    json!({"ids": ids.iter().take(CHANGE_IDS).collect::<Vec<_>>(), "more": ids.len() > CHANGE_IDS})
}

fn count(input: &Value, key: &str, default: usize) -> Result<usize, AppError> {
    match &input[key] {
        Value::Null => Ok(default),
        value => value
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| error("INVALID_ARGUMENT", format!("{key} must be a whole number"))),
    }
}

/// `scene_summary`, or one of the actions that change the saved scene (the answer is then the new
/// summary).
pub async fn run(name: &str, input: &Value) -> Result<Value, AppError> {
    let saved = saved::load().await?;
    if name == "scene_summary" {
        return Ok(actions::summary(
            &saved.scene,
            count(input, "offset", 0)?,
            count(input, "limit", 50)?,
        ));
    }
    let mut scene = saved.scene;
    actions::edit(name, &mut scene, input, &mut stamp()?)?;
    // The saved scene stays within the limit of a save from the window.
    scene::parse(&scene.to_string())?;
    let mut change = named(input);
    change["tool"] = json!(name);
    saved::save(&scene, saved.version, change).await?;
    Ok(actions::summary(&scene, 0, 50))
}

fn with_extension(name: &str) -> String {
    if name.ends_with(".excalidraw") {
        name.to_owned()
    } else {
        format!("{name}.excalidraw")
    }
}

/// `save_file`: a copy under a private name, or offered to the person when no name is given.
pub async fn save_file(input: &Value) -> Result<Value, AppError> {
    let scene = saved::live_scene(&saved::load().await?.scene);
    let name = input["name"].as_str();
    let file = name.map_or_else(
        || {
            format!(
                "diagram-{}",
                stamp().map(|mut s| s.id()).unwrap_or_default()
            )
        },
        str::to_owned,
    );
    let mut result = file_io::write(&json!({
        "name": with_extension(&file), "text": scene.to_string(), "offer": name.is_none(),
    }))
    .await?;
    result["automaticSaveError"] = Value::Null;
    Ok(result)
}

/// `open_file`: replace the saved diagram with a private file, or with the one the person picks.
pub async fn open_file(input: &Value) -> Result<Value, AppError> {
    let source = match input["name"].as_str() {
        Some(name) => file_io::read(&json!({"name": with_extension(name)})).await?,
        None => {
            let picked = file_io::pick().await?;
            let Some(grant) = picked["grants"][0].as_str() else {
                return Ok(json!({"cancelled": true}));
            };
            file_io::read(&json!({"grantId": grant})).await?
        }
    };
    let saved = saved::load().await?;
    let opened = actions::replace_scene(&saved.scene, &source["scene"], &mut stamp()?);
    scene::parse(&opened.to_string())?;
    let elements = saved::live_scene(&opened)["elements"]
        .as_array()
        .map_or(0, Vec::len);
    saved::save(
        &opened,
        saved.version,
        json!({"tool": "open_file", "elements": elements}),
    )
    .await?;
    Ok(json!({"name": source["name"], "elements": elements}))
}

/// `export_file`: an Excalidraw copy offered to the person. Pictures need the window's renderer,
/// so the person exports those from its menu.
pub async fn export_file(input: &Value) -> Result<Value, AppError> {
    if input["format"] != "json" {
        return Err(error(
            "INVALID_ARGUMENT",
            "an agent exports json; PNG and SVG are exported from the open window's menu",
        ));
    }
    let scene = saved::live_scene(&saved::load().await?.scene);
    let name = with_extension(input["name"].as_str().unwrap_or("diagram"));
    let mut result =
        file_io::write(&json!({"name": name, "text": scene.to_string(), "offer": true})).await?;
    result["automaticSaveError"] = Value::Null;
    Ok(result)
}
