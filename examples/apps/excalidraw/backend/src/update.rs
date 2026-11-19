//! `update_elements` on the saved scene: move or resize shapes and text, with the text bound in a
//! resized shape wrapped again, the text bound in a moved shape moved with it, and the arrows
//! bound to the shapes drawn again from edge to edge.

use crate::binding::boundary;
use crate::element::{self, get, num, with};
use crate::label;
use crate::stamp::Stamp;
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};
use std::collections::HashMap;

fn invalid(message: &str) -> AppError {
    error("INVALID_ARGUMENT", message)
}

fn offset(request: &Value, key: &str) -> Result<Option<f64>, AppError> {
    match &request[key] {
        Value::Null => Ok(None),
        value => value
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(|| invalid(&format!("{key} must be a number"))),
    }
}

fn centre(shape: &Value) -> (f64, f64) {
    (
        get(shape, "x") + get(shape, "width") / 2.0,
        get(shape, "y") + get(shape, "height") / 2.0,
    )
}

/// The container drawn at its new size, its label wrapped again for it and the container grown
/// if the label no longer fits: what the converter lays out for a labelled shape.
fn relayout(next: &Value, label: &Value, stamp: &mut Stamp) -> (Value, Value) {
    let mut container = next.clone();
    container["boundElements"] = json!([{"type": "text", "id": label["id"]}]);
    let mut text = label.clone();
    text["text"] = label["originalText"].clone();
    label::fit(&mut container, &mut text, stamp);
    (container, text)
}

fn point_at(arrow: &Value, index: usize) -> (f64, f64) {
    let point = &arrow["points"][index];
    (
        get(arrow, "x") + point[0].as_f64().unwrap_or(0.0),
        get(arrow, "y") + point[1].as_f64().unwrap_or(0.0),
    )
}

pub fn update(scene: &mut Value, request: &Value, stamp: &mut Stamp) -> Result<(), AppError> {
    let ids: Vec<&str> = request["ids"]
        .as_array()
        .filter(|ids| !ids.is_empty() && ids.len() <= 100)
        .and_then(|ids| ids.iter().map(Value::as_str).collect())
        .ok_or_else(|| invalid("ids must list 1 to 100 element IDs"))?;
    let (dx, dy) = (
        offset(request, "dx")?.unwrap_or(0.0),
        offset(request, "dy")?.unwrap_or(0.0),
    );
    let (width, height) = (offset(request, "width")?, offset(request, "height")?);
    let elements = scene["elements"].as_array_mut().expect("element list");
    let by_id: HashMap<String, usize> = elements
        .iter()
        .enumerate()
        .filter(|(_, e)| element::live(e))
        .filter_map(|(at, e)| e["id"].as_str().map(|id| (id.to_owned(), at)))
        .collect();
    let targets: Vec<usize> = elements
        .iter()
        .enumerate()
        .filter(|(at, e)| {
            element::live(e)
                && ids.contains(&e["id"].as_str().unwrap_or_default())
                && by_id.get(e["id"].as_str().unwrap_or_default()) == Some(at)
        })
        .map(|(at, _)| at)
        .collect();
    if targets.len() != ids.len() {
        return Err(invalid(
            "Every ID must identify an existing element. Read scene_summary first.",
        ));
    }
    if targets.iter().any(|at| {
        !matches!(
            elements[*at]["type"].as_str(),
            Some("rectangle" | "ellipse" | "diamond" | "text")
        )
    }) {
        return Err(invalid(
            "Move/resize supports shapes and text; redraw arrows with connect_elements.",
        ));
    }
    let resizes = width.is_some() || height.is_some();
    if resizes && targets.iter().any(|at| elements[*at]["type"] == "text") {
        return Err(invalid("Resize the text container, or move the text."));
    }
    let mut changed: HashMap<String, Value> = HashMap::new();
    for at in &targets {
        let e = &elements[*at];
        let mut updates = vec![("x", num(get(e, "x") + dx)), ("y", num(get(e, "y") + dy))];
        if let Some(width) = width {
            updates.push(("width", num(width)));
        }
        if let Some(height) = height {
            updates.push(("height", num(height)));
        }
        changed.insert(
            e["id"].as_str().unwrap_or_default().to_owned(),
            with(e, updates, stamp),
        );
    }
    let original = elements.clone();
    for e in original.iter().filter(|e| e["type"] == "text") {
        let Some(container) = e["containerId"].as_str() else {
            continue;
        };
        let Some(next) = changed.get(container).cloned() else {
            continue;
        };
        let Some(old) = by_id.get(container).map(|at| &original[*at]) else {
            continue;
        };
        let resized =
            get(&next, "width") != get(old, "width") || get(&next, "height") != get(old, "height");
        let id = e["id"].as_str().unwrap_or_default().to_owned();
        if resized && e["originalText"].as_str().is_some_and(|t| !t.is_empty()) {
            let (container_layout, text_layout) = relayout(&next, e, stamp);
            let grown = with(
                &next,
                vec![
                    ("width", container_layout["width"].clone()),
                    ("height", container_layout["height"].clone()),
                ],
                stamp,
            );
            changed.insert(container.to_owned(), grown);
            let text = with(
                e,
                vec![
                    ("text", text_layout["text"].clone()),
                    ("x", text_layout["x"].clone()),
                    ("y", text_layout["y"].clone()),
                    ("width", text_layout["width"].clone()),
                    ("height", text_layout["height"].clone()),
                    ("angle", text_layout["angle"].clone()),
                ],
                stamp,
            );
            changed.insert(id, text);
        } else {
            let moved = with(
                e,
                vec![
                    ("x", num(get(e, "x") + get(&next, "x") - get(old, "x"))),
                    ("y", num(get(e, "y") + get(&next, "y") - get(old, "y"))),
                ],
                stamp,
            );
            changed.insert(id, moved);
        }
    }
    for e in original.iter().filter(|e| element::is_arrow(e)) {
        let end_of = |key: &str| e[key]["elementId"].as_str().map(str::to_owned);
        let (from_id, to_id) = (end_of("startBinding"), end_of("endBinding"));
        let moved = |id: &Option<String>| id.as_ref().is_some_and(|id| changed.contains_key(id));
        if !moved(&from_id) && !moved(&to_id) {
            continue;
        }
        let current = |id: &Option<String>| {
            id.as_ref().and_then(|id| {
                changed
                    .get(id)
                    .cloned()
                    .or_else(|| by_id.get(id).map(|at| original[*at].clone()))
            })
        };
        let (from, to) = (current(&from_id), current(&to_id));
        let last = e["points"].as_array().map_or(0, Vec::len).saturating_sub(1);
        let (old_start, old_end) = (point_at(e, 0), point_at(e, last));
        let from_centre = from.as_ref().map_or(old_start, centre);
        let to_centre = to.as_ref().map_or(old_end, centre);
        let (x, y) = from
            .as_ref()
            .map_or(old_start, |from| boundary(from, to_centre));
        let (end_x, end_y) = to.as_ref().map_or(old_end, |to| boundary(to, from_centre));
        let count = e["points"].as_array().map_or(0, Vec::len);
        let points: Vec<Value> = e["points"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, point)| {
                let t = i as f64 / (count - 1) as f64;
                let (px, py) = (
                    point[0].as_f64().unwrap_or(0.0),
                    point[1].as_f64().unwrap_or(0.0),
                );
                json!([
                    num(
                        get(e, "x") + px + (x - old_start.0) * (1.0 - t) + (end_x - old_end.0) * t
                            - x
                    ),
                    num(
                        get(e, "y") + py + (y - old_start.1) * (1.0 - t) + (end_y - old_end.1) * t
                            - y
                    )
                ])
            })
            .collect();
        let size = element::size_from_points(&json!(points));
        let mut updates = vec![("x", num(x)), ("y", num(y))];
        updates.extend(size);
        updates.push(("points", json!(points)));
        let next = with(e, updates, stamp);
        let arrow_id = e["id"].as_str().unwrap_or_default().to_owned();
        for label in original
            .iter()
            .filter(|l| l["type"] == "text" && l["containerId"] == e["id"])
        {
            let moved = with(
                label,
                vec![
                    ("x", num((x + end_x - get(label, "width")) / 2.0)),
                    ("y", num((y + end_y - get(label, "height")) / 2.0)),
                ],
                stamp,
            );
            changed.insert(label["id"].as_str().unwrap_or_default().to_owned(), moved);
        }
        changed.insert(arrow_id, next);
    }
    for e in elements.iter_mut() {
        if let Some(next) = e["id"].as_str().and_then(|id| changed.remove(id)) {
            *e = next;
        }
    }
    Ok(())
}
