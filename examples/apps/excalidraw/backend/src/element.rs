//! Excalidraw scene elements as JSON objects: the constructors of `newElement`, `newTextElement`
//! and `newArrowElement`, and `mutateElement`'s change check, with the same defaults, so an
//! element made here is one the page would have made.

use crate::stamp::Stamp;
use serde_json::{json, Map, Value};

pub type Fields = Map<String, Value>;

/// A number the way `JSON.stringify` writes it: whole values carry no fraction.
pub fn num(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

pub fn get(element: &Value, key: &str) -> f64 {
    element[key].as_f64().unwrap_or(0.0)
}

pub fn fields(element: &mut Value) -> &mut Fields {
    element.as_object_mut().expect("scene elements are objects")
}

pub fn live(element: &Value) -> bool {
    element["isDeleted"] != true
}

pub fn is_arrow(element: &Value) -> bool {
    element["type"] == "arrow"
}

/// `mutateElement`: set the fields, and when one really changed, take a new version. An object
/// always counts as changed (but not `groupIds`), a list of points only when a point moved.
pub fn mutate(element: &mut Value, updates: Vec<(&str, Value)>, stamp: &mut Stamp) {
    let element = fields(element);
    let mut changed = false;
    let mut updates: Vec<(String, Value)> = updates
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    if let Some((_, points)) = updates.iter().find(|(key, _)| key == "points") {
        let size = size_from_points(points);
        for (key, value) in size {
            if !updates.iter().any(|(k, _)| k == key) {
                updates.push((key.to_owned(), value));
            }
        }
    }
    for (key, value) in updates {
        let same = match (element.get(&key), key.as_str()) {
            (Some(previous), "points") => same_points(previous, &value),
            (Some(previous), "groupIds") => *previous == value,
            (Some(previous), _) if !value.is_object() && !value.is_array() => *previous == value,
            _ => false,
        };
        if !same {
            element.insert(key, value);
            changed = true;
        }
    }
    if changed {
        stamp.bump(element);
    }
}

/// Excalidraw compares every point but the first when it asks whether the points changed.
fn same_points(previous: &Value, next: &Value) -> bool {
    match (previous.as_array(), next.as_array()) {
        (Some(a), Some(b)) if a.len() == b.len() => a.iter().zip(b).skip(1).all(|(p, q)| p == q),
        _ => false,
    }
}

/// `getSizeFromPoints`.
pub fn size_from_points(points: &Value) -> Vec<(&'static str, Value)> {
    let coordinates = |axis: usize| -> Vec<f64> {
        points
            .as_array()
            .into_iter()
            .flatten()
            .map(|point| point[axis].as_f64().unwrap_or(0.0))
            .collect()
    };
    let extent = |axis: usize| {
        let values = coordinates(axis);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        num(max - min)
    };
    vec![("width", extent(0)), ("height", extent(1))]
}

/// `newElementWith`: a changed copy with the next version, or the element itself when none of the
/// updates changes anything.
pub fn with(element: &Value, updates: Vec<(&str, Value)>, stamp: &mut Stamp) -> Value {
    let mut next = element.clone();
    let changed = updates.iter().any(|(key, value)| {
        let same = element.get(*key) == Some(value) && !value.is_object() && !value.is_array();
        !same
    });
    if !changed {
        return next;
    }
    let fields = fields(&mut next);
    for (key, value) in updates {
        fields.insert(key.to_owned(), value);
    }
    stamp.bump(fields);
    next
}

/// What `newElement` takes besides the type; anything left out has Excalidraw's default.
#[derive(Default)]
pub struct Base {
    pub id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub stroke_color: Option<Value>,
}

pub fn new_base(kind: &str, base: Base, stamp: &mut Stamp) -> Fields {
    let id = base.id.unwrap_or_else(|| stamp.id());
    let seed = stamp.integer();
    let element = json!({
        "id": id, "type": kind, "x": num(base.x), "y": num(base.y),
        "width": num(base.width), "height": num(base.height), "angle": 0,
        "strokeColor": base.stroke_color.unwrap_or_else(|| json!("#1e1e1e")),
        "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 2,
        "strokeStyle": "solid", "roughness": 1, "opacity": 100, "groupIds": [], "frameId": null,
        "index": null, "roundness": null, "seed": seed, "version": 1, "versionNonce": 0,
        "isDeleted": false, "boundElements": null, "updated": stamp.now_ms, "link": null,
        "locked": false,
    });
    match element {
        Value::Object(fields) => fields,
        _ => unreachable!("a JSON object literal"),
    }
}

/// A rectangle, ellipse or diamond.
pub fn new_shape(kind: &str, base: Base, stamp: &mut Stamp) -> Value {
    Value::Object(new_base(kind, base, stamp))
}

/// A straight arrow whose `points` are relative to its position; the size is the points'.
pub fn new_arrow(base: Base, points: Value, stamp: &mut Stamp) -> Value {
    let mut arrow = new_base("arrow", base, stamp);
    for (key, value) in size_from_points(&points) {
        arrow.insert(key.into(), value);
    }
    arrow.insert("points".into(), points);
    arrow.insert("lastCommittedPoint".into(), Value::Null);
    arrow.insert("startBinding".into(), Value::Null);
    arrow.insert("endBinding".into(), Value::Null);
    arrow.insert("startArrowhead".into(), Value::Null);
    arrow.insert("endArrowhead".into(), json!("arrow"));
    arrow.insert("elbowed".into(), json!(false));
    Value::Object(arrow)
}

pub struct TextProps {
    pub text: String,
    pub font_size: f64,
    pub text_align: &'static str,
    pub vertical_align: &'static str,
    pub container_id: Option<String>,
}

/// `newTextElement` with the Virgil face: `base.x` and `base.y` are where the text's anchor goes
/// (its centre or its right end for centred or right-aligned text).
pub fn new_text(base: Base, props: TextProps, stamp: &mut Stamp) -> Value {
    let text = crate::text::normalize(&props.text);
    let (width, height) = crate::text::measure(&text, props.font_size, crate::text::LINE_HEIGHT);
    let x = base.x
        - match props.text_align {
            "center" => width / 2.0,
            "right" => width,
            _ => 0.0,
        };
    let y = base.y
        - if props.vertical_align == "middle" {
            height / 2.0
        } else {
            0.0
        };
    let base = Base {
        x,
        y,
        width,
        height,
        ..base
    };
    let mut element = new_base("text", base, stamp);
    element.insert("text".into(), json!(text));
    element.insert("fontSize".into(), num(props.font_size));
    element.insert("fontFamily".into(), json!(1));
    element.insert("textAlign".into(), json!(props.text_align));
    element.insert("verticalAlign".into(), json!(props.vertical_align));
    element.insert("containerId".into(), json!(props.container_id));
    element.insert("originalText".into(), json!(text));
    element.insert("autoResize".into(), json!(true));
    element.insert("lineHeight".into(), json!(crate::text::LINE_HEIGHT));
    Value::Object(element)
}
