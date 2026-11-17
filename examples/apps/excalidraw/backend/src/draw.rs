//! `add_elements` and `connect_elements` on the saved scene: the elements the page's converter
//! (`convertToExcalidrawElements`) would have made from the same request, with the same fields,
//! sizes, bound text and arrow bindings.

use crate::binding;
use crate::element::{self, get, mutate, new_arrow, new_shape, new_text, num, Base, TextProps};
use crate::label;
use crate::stamp::Stamp;
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::{json, Value};

const DEFAULT_WIDTH: f64 = 160.0;
const DEFAULT_HEIGHT: f64 = 90.0;
const DEFAULT_ARROW_LENGTH: f64 = 160.0;
/// Bound arrows start and end this far inside their points so a binding does not sit on the edge.
const ARROW_INSET: f64 = 0.5;

fn invalid(message: impl Into<String>) -> AppError {
    error("INVALID_ARGUMENT", message)
}

fn number(request: &Value, key: &str) -> Result<Option<f64>, AppError> {
    match &request[key] {
        Value::Null => Ok(None),
        value => value
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(|| invalid(format!("{key} must be a number"))),
    }
}

fn text_of(request: &Value, key: &str) -> Result<Option<String>, AppError> {
    match &request[key] {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(invalid(format!("{key} must be a string"))),
    }
}

/// Excalidraw's indices are keys that sort in order of the elements; this is the key that sorts
/// right after `previous`, or `None` when the integer part cannot grow.
fn key_after(previous: Option<&str>) -> Option<String> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let Some(previous) = previous else {
        return Some("a0".into());
    };
    let head = *previous.as_bytes().first()?;
    let length = match head {
        b'a'..=b'z' => (head - b'a') as usize + 2,
        b'A'..=b'Z' => (b'Z' - head) as usize + 2,
        _ => return None,
    };
    let integer = previous.get(..length)?;
    let mut digits: Vec<u8> = integer.as_bytes()[1..].to_vec();
    let mut carry = true;
    for digit in digits.iter_mut().rev() {
        let next = DIGITS.iter().position(|d| d == digit)? + 1;
        if next == DIGITS.len() {
            *digit = DIGITS[0];
        } else {
            *digit = DIGITS[next];
            carry = false;
            break;
        }
    }
    let mut head = head;
    if carry {
        match head {
            b'Z' => return Some("a0".into()),
            b'z' => return None,
            _ => {}
        }
        head += 1;
        if head > b'a' {
            digits.push(DIGITS[0]);
        } else {
            digits.pop();
        }
    }
    let mut key = String::from(head as char);
    key.push_str(std::str::from_utf8(&digits).ok()?);
    Some(key)
}

/// Give the elements that have no index one each, after the last element that has one.
fn index_new(elements: &mut [Value], stamp: &mut Stamp) {
    let mut previous: Option<String> = None;
    for element in elements.iter_mut() {
        match element["index"].as_str() {
            Some(index) => previous = Some(index.to_owned()),
            None => {
                if let Some(index) = key_after(previous.as_deref()) {
                    mutate(element, vec![("index", json!(index))], stamp);
                    previous = element["index"].as_str().map(str::to_owned);
                }
            }
        }
    }
}

/// An arrow whose ends are nudged inward, as the converter does for every arrow it makes.
fn inset_points(arrow: &mut Value) {
    let points = arrow["points"].as_array().cloned().unwrap_or_default();
    let Some(end) = points.len().checked_sub(1).filter(|end| *end > 0) else {
        return;
    };
    let coordinate = |point: usize, axis: usize| points[point][axis].as_f64().unwrap_or(0.0);
    let mut next = points.clone();
    let mut set = |point: usize, axis: usize, value: f64| next[point][axis] = num(value);
    for axis in 0..2 {
        let (before, last) = (coordinate(end - 1, axis), coordinate(end, axis));
        if last > before {
            set(0, axis, ARROW_INSET);
            set(end, axis, last - ARROW_INSET);
        } else if last < before {
            set(0, axis, -ARROW_INSET);
            set(end, axis, last + ARROW_INSET);
        }
    }
    arrow["points"] = json!(next);
}

fn draw_one(request: &Value, stamp: &mut Stamp) -> Result<(Value, Option<Value>), AppError> {
    let kind = request["type"]
        .as_str()
        .ok_or_else(|| invalid("every element needs a type"))?;
    let (x, y) = (
        number(request, "x")?.ok_or_else(|| invalid("every element needs x and y"))?,
        number(request, "y")?.ok_or_else(|| invalid("every element needs x and y"))?,
    );
    let id = text_of(request, "id")?;
    let label = text_of(request, "label")?.filter(|label| !label.is_empty());
    let base = |width: f64, height: f64| Base {
        id: id.clone(),
        x,
        y,
        width,
        height,
        ..Base::default()
    };
    match kind {
        "text" => {
            let text = text_of(request, "text")?.or(label).unwrap_or_default();
            let props = TextProps {
                text,
                font_size: 20.0,
                text_align: "left",
                vertical_align: "top",
                container_id: None,
            };
            Ok((new_text(base(0.0, 0.0), props, stamp), None))
        }
        "arrow" => {
            let end_x = number(request, "endX")?.unwrap_or(x + DEFAULT_ARROW_LENGTH);
            let end_y = number(request, "endY")?.unwrap_or(y);
            let points = json!([[0, 0], [num(end_x - x), num(end_y - y)]]);
            let mut arrow = new_arrow(base(0.0, 0.0), points, stamp);
            let text = label.map(|label| label::bind(&mut arrow, &label, stamp));
            inset_points(&mut arrow);
            Ok((arrow, text))
        }
        "rectangle" | "ellipse" | "diamond" => {
            let width = number(request, "width")?.unwrap_or(DEFAULT_WIDTH);
            let height = number(request, "height")?.unwrap_or(DEFAULT_HEIGHT);
            let mut shape = new_shape(kind, base(width, height), stamp);
            let text = label.map(|label| label::bind(&mut shape, &label, stamp));
            Ok((shape, text))
        }
        other => Err(invalid(format!("cannot draw an element of type {other}"))),
    }
}

/// Put the elements of the request at the end of the scene, each followed by nothing and all the
/// bound text after them, in the order the converter returns them.
pub fn add(scene: &mut Value, request: &Value, stamp: &mut Stamp) -> Result<(), AppError> {
    let requested = request["elements"]
        .as_array()
        .filter(|elements| !elements.is_empty() && elements.len() <= 50)
        .ok_or_else(|| invalid("elements must list 1 to 50 elements"))?;
    let elements = scene["elements"].as_array_mut().expect("element list");
    let mut known: std::collections::HashSet<String> = elements
        .iter()
        .filter(|e| element::live(e))
        .filter_map(|e| e["id"].as_str().map(str::to_owned))
        .collect();
    let (mut shapes, mut labels) = (Vec::new(), Vec::new());
    for request in requested {
        if let Some(id) = request["id"].as_str() {
            if !known.insert(id.to_owned()) {
                return Err(invalid(format!("Element ID already exists: {id}")));
            }
        }
        let (shape, text) = draw_one(request, stamp)?;
        shapes.push(shape);
        labels.extend(text);
    }
    // An ID of an element that was deleted may be used again.
    let reused: Vec<String> = shapes
        .iter()
        .chain(&labels)
        .filter_map(|e| e["id"].as_str().map(str::to_owned))
        .collect();
    elements.retain(|e| {
        !e["id"]
            .as_str()
            .is_some_and(|id| reused.iter().any(|r| r == id))
    });
    elements.extend(shapes);
    elements.extend(labels);
    index_new(elements, stamp);
    Ok(())
}

/// The point on the edge of `shape` that faces `towards`, and the point of `towards` that faces back.
fn facing_edges(source: &Value, target: &Value) -> ((f64, f64), (f64, f64)) {
    let centre = |s: &Value| {
        (
            get(s, "x") + get(s, "width") / 2.0,
            get(s, "y") + get(s, "height") / 2.0,
        )
    };
    let (a, b) = (centre(source), centre(target));
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (sx, sy, sw, sh) = (
        get(source, "x"),
        get(source, "y"),
        get(source, "width"),
        get(source, "height"),
    );
    let (tx, ty, tw, th) = (
        get(target, "x"),
        get(target, "y"),
        get(target, "width"),
        get(target, "height"),
    );
    if dx.abs() >= dy.abs() {
        (
            (if dx >= 0.0 { sx + sw } else { sx }, sy + sh / 2.0),
            (if dx >= 0.0 { tx } else { tx + tw }, ty + th / 2.0),
        )
    } else {
        (
            (sx + sw / 2.0, if dy >= 0.0 { sy + sh } else { sy }),
            (tx + tw / 2.0, if dy >= 0.0 { ty } else { ty + th }),
        )
    }
}

/// `bindLinearElement` for one end of `arrow`: store the binding, and list the arrow in the shape.
fn bind_end(arrow: &mut Value, shape: &mut Value, end: &str, stamp: &mut Stamp) {
    let points = arrow["points"].as_array().cloned().unwrap_or_default();
    let global = |index: usize| {
        (
            get(arrow, "x") + points[index][0].as_f64().unwrap_or(0.0),
            get(arrow, "y") + points[index][1].as_f64().unwrap_or(0.0),
        )
    };
    let (edge, adjacent) = if end == "start" {
        (global(0), global(points.len() - 1))
    } else {
        (global(points.len() - 1), global(0))
    };
    let (focus, gap) = binding::bind(shape, adjacent, edge);
    let key = if end == "start" {
        "startBinding"
    } else {
        "endBinding"
    };
    mutate(
        arrow,
        vec![(
            key,
            json!({"elementId": shape["id"], "focus": num(focus), "gap": num(gap)}),
        )],
        stamp,
    );
    let mut bound = shape["boundElements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !bound.iter().any(|b| b["id"] == arrow["id"]) {
        bound.push(json!({"id": arrow["id"], "type": "arrow"}));
        mutate(shape, vec![("boundElements", json!(bound))], stamp);
    }
}

pub fn connect(scene: &mut Value, request: &Value, stamp: &mut Stamp) -> Result<(), AppError> {
    let (from, to) = (
        request["from"]
            .as_str()
            .ok_or_else(|| invalid("from must be an element ID"))?,
        request["to"]
            .as_str()
            .ok_or_else(|| invalid("to must be an element ID"))?,
    );
    if from == to {
        return Err(invalid("Choose two distinct shapes."));
    }
    let label = text_of(request, "label")?.filter(|label| !label.is_empty());
    let elements = scene["elements"].as_array_mut().expect("element list");
    let position = |id: &str| {
        elements
            .iter()
            .position(|e| element::live(e) && e["id"].as_str() == Some(id))
            .ok_or_else(|| {
                invalid("Every ID must identify an existing element. Read scene_summary first.")
            })
    };
    let (from_at, to_at) = (position(from)?, position(to)?);
    for at in [from_at, to_at] {
        if !matches!(
            elements[at]["type"].as_str(),
            Some("rectangle" | "ellipse" | "diamond")
        ) {
            return Err(invalid("Connect rectangle, ellipse or diamond shapes."));
        }
    }
    let (start, end) = facing_edges(&elements[from_at], &elements[to_at]);
    let points = json!([[0, 0], [num(end.0 - start.0), num(end.1 - start.1)]]);
    let id = stamp.id();
    let mut arrow = new_arrow(
        Base {
            id: Some(id),
            x: start.0,
            y: start.1,
            ..Base::default()
        },
        points,
        stamp,
    );
    let text = label.map(|label| label::bind(&mut arrow, &label, stamp));
    // The shapes come back as copies of themselves, bound to the arrow.
    for (at, edge) in [(from_at, "start"), (to_at, "end")] {
        let mut shape = elements[at].clone();
        shape["updated"] = json!(stamp.now_ms);
        bind_end(&mut arrow, &mut shape, edge, stamp);
        elements[at] = shape;
    }
    inset_points(&mut arrow);
    elements.push(arrow);
    elements.extend(text);
    index_new(elements, stamp);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_continue_after_the_last_one_and_carry_over() {
        assert_eq!(key_after(None).as_deref(), Some("a0"));
        assert_eq!(key_after(Some("a0")).as_deref(), Some("a1"));
        assert_eq!(key_after(Some("a9")).as_deref(), Some("aA"));
        assert_eq!(key_after(Some("aZ")).as_deref(), Some("aa"));
        assert_eq!(key_after(Some("az")).as_deref(), Some("b00"));
        assert_eq!(key_after(Some("a0V")).as_deref(), Some("a1"));
        assert_eq!(key_after(Some(&"z".repeat(27))), None);
    }
}
