//! The diagram actions on the saved scene, run by the backend so an agent can act on a diagram
//! with no page open: `scene_summary`, `select_elements`, `delete_elements` and `group_elements`
//! here, `add_elements` and `connect_elements` in `draw.rs`, `update_elements` in `update.rs`.

use crate::stamp::Stamp;
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

const SUMMARY_BYTES: usize = 48 * 1024;
const UNKNOWN_ID: &str = "Every ID must identify an existing element. Read scene_summary first.";

use crate::element::{fields, live};

/// Change an element and give it its next version.
fn touch(stamp: &mut Stamp, element: &mut Value, patch: impl FnOnce(&mut Map<String, Value>)) {
    let fields = fields(element);
    patch(fields);
    stamp.bump(fields);
}

/// How long a deleted element stays in the saved scene: a window that has not yet seen the
/// deletion merges it, instead of drawing the element again.
const TOMBSTONE_MS: u64 = 60 * 60 * 1000;

/// Drop the deleted elements that every open window has had time to see.
pub fn forget_old_deletions(scene: &mut Value, now_ms: u64) {
    elements_mut(scene).retain(|e| {
        live(e) || now_ms.saturating_sub(e["updated"].as_u64().unwrap_or(now_ms)) < TOMBSTONE_MS
    });
}

fn text_of(value: &Value, limit: usize) -> (String, bool) {
    let text = value.as_str().unwrap_or_default();
    (
        text.chars().take(limit).collect(),
        text.chars().count() > limit,
    )
}

fn elements_mut(scene: &mut Value) -> &mut Vec<Value> {
    scene["elements"]
        .as_array_mut()
        .expect("a parsed scene has an element list")
}

fn existing(scene: &Value, ids: &[String]) -> Result<(), AppError> {
    let found = scene["elements"]
        .as_array()
        .expect("element list")
        .iter()
        .filter(|e| {
            live(e)
                && e["id"]
                    .as_str()
                    .is_some_and(|id| ids.iter().any(|i| i == id))
        })
        .count();
    if found != ids.len() {
        return Err(error("INVALID_ARGUMENT", UNKNOWN_ID));
    }
    Ok(())
}

pub fn ids(input: &Value) -> Result<Vec<String>, AppError> {
    input["ids"]
        .as_array()
        .filter(|ids| !ids.is_empty() && ids.len() <= 100)
        .and_then(|ids| {
            ids.iter()
                .map(|id| id.as_str().filter(|id| !id.is_empty()).map(str::to_owned))
                .collect()
        })
        .ok_or_else(|| error("INVALID_ARGUMENT", "ids must list 1 to 100 element IDs"))
}

/// A bounded view of the live elements and the selection.
pub fn summary(scene: &Value, offset: usize, limit: usize) -> Value {
    let all: Vec<&Value> = scene["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| live(e))
        .collect();
    let mut elements = Vec::new();
    let mut bytes = 0;
    for element in all.iter().skip(offset).take(limit) {
        let groups = element["groupIds"].as_array().cloned().unwrap_or_default();
        let mut entry = json!({
            "id": element["id"], "type": element["type"], "x": element["x"], "y": element["y"],
            "width": element["width"], "height": element["height"], "angle": element["angle"],
            "groups": groups.iter().take(16).collect::<Vec<_>>(), "groupsTruncated": groups.len() > 16,
            "containerId": element["containerId"].as_str().map_or(Value::Null, |id| json!(id)),
        });
        if element["type"] == "text" {
            let (text, truncated) = text_of(&element["text"], 500);
            entry["text"] = json!(text);
            entry["textTruncated"] = json!(truncated);
        }
        if let Some(points) = element["points"].as_array() {
            entry["points"] = json!(points.iter().take(4).collect::<Vec<_>>());
            entry["pointsTruncated"] = json!(points.len() > 4);
        }
        if element["type"] == "arrow" {
            let bound = |binding: &Value| {
                binding["elementId"]
                    .as_str()
                    .map_or(Value::Null, |id| json!(id))
            };
            entry["from"] = bound(&element["startBinding"]);
            entry["to"] = bound(&element["endBinding"]);
        }
        let size = entry.to_string().len();
        if bytes + size > SUMMARY_BYTES {
            break;
        }
        bytes += size;
        elements.push(entry);
    }
    let next = offset + elements.len();
    json!({
        "total": all.len(), "offset": offset,
        "nextOffset": if next < all.len() { json!(next) } else { Value::Null },
        "selected": scene["appState"]["selectedElementIds"].as_object().into_iter().flatten().take(100)
            .map(|(id, _)| id).collect::<Vec<_>>(),
        "elements": elements,
    })
}

pub fn select(scene: &mut Value, ids: &[String]) -> Result<(), AppError> {
    existing(scene, ids)?;
    scene["appState"]["selectedElementIds"] =
        Value::Object(ids.iter().map(|id| (id.clone(), json!(true))).collect());
    Ok(())
}

/// Delete the elements, the text bound in them, and every binding that pointed at them. The
/// deleted elements stay in the scene, marked, until `forget_old_deletions`.
pub fn delete(scene: &mut Value, ids: &[String], stamp: &mut Stamp) -> Result<(), AppError> {
    existing(scene, ids)?;
    let mut removed: HashSet<String> = ids.iter().cloned().collect();
    for element in elements_mut(scene).iter().filter(|e| live(e)) {
        if element["containerId"]
            .as_str()
            .is_some_and(|id| removed.contains(id))
        {
            removed.insert(element["id"].as_str().unwrap_or_default().to_owned());
        }
    }
    let was_removed = |value: &Value| value.as_str().is_some_and(|id| removed.contains(id));
    for element in elements_mut(scene).iter_mut().filter(|e| live(e)) {
        let gone = element["id"]
            .as_str()
            .is_some_and(|id| removed.contains(id));
        let arrow = element["type"] == "arrow";
        touch(stamp, element, |fields| {
            if gone {
                fields.insert("isDeleted".into(), json!(true));
                return;
            }
            let bound = match fields.get("boundElements").and_then(Value::as_array) {
                Some(bound) => json!(bound
                    .iter()
                    .filter(|b| !was_removed(&b["id"]))
                    .collect::<Vec<_>>()),
                None => Value::Null,
            };
            fields.insert("boundElements".into(), bound);
            if arrow {
                for key in ["startBinding", "endBinding"] {
                    if was_removed(&fields[key]["elementId"]) {
                        fields.insert(key.into(), Value::Null);
                    }
                }
            }
        });
    }
    scene["appState"]["selectedElementIds"] = json!({});
    Ok(())
}

/// The diagram `opened` in place of the one in `current`: its elements, and the elements it
/// replaces marked deleted, so a window that merges the saved scene drops them instead of
/// keeping them. An element that is in both carries on from the version it had.
pub fn replace_scene(current: &Value, opened: &Value, stamp: &mut Stamp) -> Value {
    let versions: std::collections::HashMap<&str, u64> = current["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| live(e))
        .filter_map(|e| Some((e["id"].as_str()?, e["version"].as_u64()?)))
        .collect();
    let opened_elements: Vec<Value> = opened["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| live(e))
        .cloned()
        .map(|mut element| {
            let earlier = element["id"].as_str().and_then(|id| versions.get(id));
            if let Some(earlier) = earlier {
                let fields = fields(&mut element);
                let own = fields.get("version").and_then(Value::as_u64).unwrap_or(0);
                fields.insert("version".into(), json!(own.max(*earlier)));
                stamp.bump(fields);
            }
            element
        })
        .collect();
    let kept: HashSet<&str> = opened_elements
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    let mut elements = opened_elements.clone();
    for element in current["elements"].as_array().into_iter().flatten() {
        if live(element) && !element["id"].as_str().is_some_and(|id| kept.contains(id)) {
            let mut gone = element.clone();
            touch(stamp, &mut gone, |fields| {
                fields.insert("isDeleted".into(), json!(true));
            });
            elements.push(gone);
        }
    }
    let mut next = opened.clone();
    next["elements"] = json!(elements);
    next
}

/// The change `name` makes to the scene; the actions that only read it are `summary`.
pub fn edit(
    name: &str,
    scene: &mut Value,
    input: &Value,
    stamp: &mut Stamp,
) -> Result<(), AppError> {
    forget_old_deletions(scene, stamp.now_ms);
    match name {
        "select_elements" => select(scene, &ids(input)?),
        "delete_elements" => delete(scene, &ids(input)?, stamp),
        "group_elements" => {
            let id = format!("group-{}", stamp.id());
            group(scene, &ids(input)?, &id, stamp)
        }
        "add_elements" => crate::draw::add(scene, input, stamp),
        "connect_elements" => crate::draw::connect(scene, input, stamp),
        "update_elements" => crate::update::update(scene, input, stamp),
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

/// Put the elements, and the text bound in them, in one new group.
pub fn group(
    scene: &mut Value,
    ids: &[String],
    group: &str,
    stamp: &mut Stamp,
) -> Result<(), AppError> {
    existing(scene, ids)?;
    let member = |element: &Value| {
        [&element["id"], &element["containerId"]]
            .iter()
            .any(|id| id.as_str().is_some_and(|id| ids.iter().any(|i| i == id)))
    };
    for element in elements_mut(scene).iter_mut().filter(|e| live(e)) {
        if member(element) {
            touch(stamp, element, |fields| {
                let mut groups = fields
                    .get("groupIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                groups.push(json!(group));
                fields.insert("groupIds".into(), json!(groups));
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: &str, kind: &str) -> Value {
        json!({"id": id, "type": kind, "x": 0, "y": 0, "width": 10, "height": 10, "angle": 0,
            "groupIds": [], "version": 1, "versionNonce": 1, "updated": 1, "isDeleted": false,
            "boundElements": null, "containerId": null})
    }

    fn scene() -> Value {
        let mut label = element("label", "text");
        label["containerId"] = json!("box");
        label["text"] = json!("hello");
        let mut box_ = element("box", "rectangle");
        box_["boundElements"] =
            json!([{"id": "label", "type": "text"}, {"id": "arrow", "type": "arrow"}]);
        let mut arrow = element("arrow", "arrow");
        arrow["points"] = json!([[0, 0], [5, 5]]);
        arrow["startBinding"] = json!({"elementId": "box", "focus": 0, "gap": 1});
        arrow["endBinding"] = json!({"elementId": "other", "focus": 0, "gap": 1});
        let mut other = element("other", "ellipse");
        other["boundElements"] = json!([{"id": "arrow", "type": "arrow"}]);
        json!({"type": "excalidraw", "version": 2, "elements": [box_, label, arrow, other],
            "appState": {"selectedElementIds": {"box": true}}, "files": {}})
    }

    fn stamp() -> Stamp {
        Stamp::new(7, 1_000)
    }

    fn strings(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn the_summary_lists_live_elements_with_their_bindings_and_pages() {
        let mut scene = scene();
        scene["elements"][3]["isDeleted"] = json!(true);
        let all = summary(&scene, 0, 50);
        assert_eq!(all["total"], 3);
        assert_eq!(all["nextOffset"], Value::Null);
        assert_eq!(all["selected"], json!(["box"]));
        assert_eq!(all["elements"][1]["text"], "hello");
        assert_eq!(all["elements"][1]["containerId"], "box");
        assert_eq!(all["elements"][2]["from"], "box");
        assert_eq!(all["elements"][2]["to"], "other");
        assert_eq!(all["elements"][2]["pointsTruncated"], false);
        let page = summary(&scene, 1, 1);
        assert_eq!(page["elements"].as_array().unwrap().len(), 1);
        assert_eq!(page["nextOffset"], 2);
    }

    #[test]
    fn selecting_names_existing_elements_only() {
        let mut scene = scene();
        select(&mut scene, &strings(&["other", "label"])).unwrap();
        assert_eq!(
            scene["appState"]["selectedElementIds"],
            json!({"other": true, "label": true})
        );
        let before = scene.clone();
        assert!(select(&mut scene, &strings(&["other", "nope"])).is_err());
        assert_eq!(scene, before, "a refused action changes nothing");
    }

    fn live_ids(scene: &Value) -> Vec<&str> {
        scene["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| live(e))
            .map(|e| e["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn deleting_takes_the_bound_text_and_clears_bindings_that_pointed_at_it() {
        let mut scene = scene();
        delete(&mut scene, &strings(&["box"]), &mut stamp()).unwrap();
        assert_eq!(
            live_ids(&scene),
            ["arrow", "other"],
            "the box and its label are deleted"
        );
        let deleted: Vec<&Value> = scene["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| !live(e))
            .collect();
        assert_eq!(
            deleted
                .iter()
                .map(|e| e["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["box", "label"],
            "they stay in the scene, marked, for a window that has not seen the deletion"
        );
        assert_eq!(deleted[0]["version"], 2);
        let arrow = &scene["elements"][2];
        assert_eq!(arrow["startBinding"], Value::Null);
        assert_eq!(arrow["endBinding"]["elementId"], "other");
        assert_eq!(arrow["version"], 2);
        assert_eq!(arrow["updated"], 1_000);
        assert_eq!(scene["appState"]["selectedElementIds"], json!({}));
        assert!(delete(&mut scene, &strings(&["box"]), &mut stamp()).is_err());
    }

    #[test]
    fn a_deletion_is_forgotten_after_an_hour() {
        let mut scene = scene();
        delete(&mut scene, &strings(&["box"]), &mut stamp()).unwrap();
        forget_old_deletions(&mut scene, 1_000 + TOMBSTONE_MS - 1);
        assert_eq!(scene["elements"].as_array().unwrap().len(), 4);
        forget_old_deletions(&mut scene, 1_000 + TOMBSTONE_MS);
        assert_eq!(live_ids(&scene), ["arrow", "other"]);
        assert_eq!(scene["elements"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn deleting_a_target_unbinds_it_from_the_elements_that_list_it() {
        let mut scene = scene();
        delete(&mut scene, &strings(&["arrow"]), &mut stamp()).unwrap();
        assert_eq!(
            scene["elements"][0]["boundElements"],
            json!([{"id": "label", "type": "text"}])
        );
        assert_eq!(scene["elements"][3]["boundElements"], json!([]));
    }

    #[test]
    fn grouping_adds_one_group_to_the_elements_and_their_bound_text() {
        let mut scene = scene();
        group(&mut scene, &strings(&["box", "other"]), "g1", &mut stamp()).unwrap();
        let groups = |index: usize| scene["elements"][index]["groupIds"].clone();
        assert_eq!(
            (groups(0), groups(1), groups(2), groups(3)),
            (json!(["g1"]), json!(["g1"]), json!([]), json!(["g1"]))
        );
        assert_eq!(
            scene["elements"][2]["version"], 1,
            "an element outside the group is not touched"
        );
        assert_ne!(scene["elements"][0]["versionNonce"], 1);
    }

    #[test]
    fn ids_are_checked_once_at_the_boundary() {
        assert_eq!(ids(&json!({"ids": ["a", "b"]})).unwrap(), ["a", "b"]);
        for bad in [
            json!({}),
            json!({"ids": []}),
            json!({"ids": [""]}),
            json!({"ids": [1]}),
            json!({"ids": vec!["a"; 101]}),
        ] {
            assert!(ids(&bad).is_err(), "{bad}");
        }
    }
}
