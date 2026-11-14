//! The text bound in a container, laid out the way Excalidraw's `redrawTextBoundingBox` does:
//! wrapped to the room the container leaves, the container grown when the text does not fit, and
//! the text placed in the container (or on the middle of an arrow).

use crate::element::{self, get, is_arrow, mutate, num, Base, TextProps};
use crate::stamp::Stamp;
use crate::text;
use serde_json::{json, Value};

const PADDING: f64 = 5.0;
const ARROW_LABEL_WIDTH_FRACTION: f64 = 0.7;
const ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO: f64 = 11.0;

/// A rectangle the text may fill: shapes leave room by how much of them a rectangle can inscribe.
fn max_width(container: &Value, label: &Value) -> f64 {
    let width = get(container, "width");
    if is_arrow(container) {
        let minimum = get(label, "fontSize") * ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO;
        return (ARROW_LABEL_WIDTH_FRACTION * width).max(minimum);
    }
    match container["type"].as_str() {
        Some("ellipse") => (width / 2.0 * 2f64.sqrt()).round() - PADDING * 2.0,
        Some("diamond") => (width / 2.0).round() - PADDING * 2.0,
        _ => width - PADDING * 2.0,
    }
}

fn max_height(container: &Value, label: &Value) -> f64 {
    let height = get(container, "height");
    if is_arrow(container) {
        return if height - PADDING * 8.0 * 2.0 <= 0.0 {
            get(label, "height")
        } else {
            height
        };
    }
    match container["type"].as_str() {
        Some("ellipse") => (height / 2.0 * 2f64.sqrt()).round() - PADDING * 2.0,
        Some("diamond") => (height / 2.0).round() - PADDING * 2.0,
        _ => height - PADDING * 2.0,
    }
}

/// The size a container needs for text of `dimension`.
fn container_dimension(dimension: f64, kind: &str) -> f64 {
    let dimension = dimension.ceil();
    let padding = PADDING * 2.0;
    match kind {
        "ellipse" => ((dimension + padding) / 2f64.sqrt() * 2.0).round(),
        "arrow" => dimension + padding * 8.0,
        "diamond" => 2.0 * (dimension + padding),
        _ => dimension + padding,
    }
}

/// Where a shape's text starts: inside the padding, and inside the inscribed rectangle of an
/// ellipse or a diamond.
fn container_origin(container: &Value) -> (f64, f64) {
    let (mut dx, mut dy) = (PADDING, PADDING);
    let (width, height) = (get(container, "width"), get(container, "height"));
    match container["type"].as_str() {
        Some("ellipse") => {
            dx += width / 2.0 * (1.0 - 2f64.sqrt() / 2.0);
            dy += height / 2.0 * (1.0 - 2f64.sqrt() / 2.0);
        }
        Some("diamond") => {
            dx += width / 4.0;
            dy += height / 4.0;
        }
        _ => {}
    }
    (get(container, "x") + dx, get(container, "y") + dy)
}

fn arrow_midpoint(arrow: &Value) -> (f64, f64) {
    let points: Vec<(f64, f64)> = arrow["points"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                get(arrow, "x") + p[0].as_f64().unwrap_or(0.0),
                get(arrow, "y") + p[1].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    match points.len() {
        0 => (get(arrow, "x"), get(arrow, "y")),
        n if n % 2 == 1 => points[n / 2],
        n => {
            let (a, b) = (points[n / 2 - 1], points[n / 2]);
            ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
        }
    }
}

fn position(container: &Value, label: &Value) -> (f64, f64) {
    if is_arrow(container) {
        let (x, y) = arrow_midpoint(container);
        return (
            x - get(label, "width") / 2.0,
            y - get(label, "height") / 2.0,
        );
    }
    let (x0, y0) = container_origin(container);
    let (room_width, room_height) = (max_width(container, label), max_height(container, label));
    let y = match label["verticalAlign"].as_str() {
        Some("top") => y0,
        Some("bottom") => y0 + (room_height - get(label, "height")),
        _ => y0 + (room_height / 2.0 - get(label, "height") / 2.0),
    };
    let x = match label["textAlign"].as_str() {
        Some("left") => x0,
        Some("right") => x0 + (room_width - get(label, "width")),
        _ => x0 + (room_width / 2.0 - get(label, "width") / 2.0),
    };
    (x, y)
}

/// `redrawTextBoundingBox` of a text bound in `container`.
pub fn fit(container: &mut Value, label: &mut Value, stamp: &mut Stamp) {
    let size = get(label, "fontSize");
    let line_height = get(label, "lineHeight");
    let wrapped = text::wrap(
        label["originalText"].as_str().unwrap_or_default(),
        size,
        max_width(container, label),
    );
    let (width, height) = text::measure(&wrapped, size, line_height);
    let mut updates = vec![
        ("text", json!(wrapped)),
        ("width", num(width)),
        ("height", num(height)),
        ("angle", container["angle"].clone()),
    ];
    if !is_arrow(container) && height > max_height(container, label) {
        let kind = container["type"].as_str().unwrap_or("rectangle");
        mutate(
            container,
            vec![("height", num(container_dimension(height, kind)))],
            stamp,
        );
    }
    if width > max_width(container, label) {
        let kind = container["type"].as_str().unwrap_or("rectangle");
        mutate(
            container,
            vec![("width", num(container_dimension(width, kind)))],
            stamp,
        );
    }
    // The position is worked out from the text as it will be, in the container as it now is.
    let mut next = label.clone();
    for (key, value) in &updates {
        next[*key] = value.clone();
    }
    let (x, y) = position(container, &next);
    updates.push(("x", num(x)));
    updates.push(("y", num(y)));
    mutate(label, updates, stamp);
}

/// `bindTextToContainer`: a label centred in `container`, bound to it and laid out.
pub fn bind(container: &mut Value, text: &str, stamp: &mut Stamp) -> Value {
    let mut label = element::new_text(
        Base {
            stroke_color: Some(container["strokeColor"].clone()),
            ..Base::default()
        },
        TextProps {
            text: text.to_owned(),
            font_size: 20.0,
            text_align: "center",
            vertical_align: "middle",
            container_id: container["id"].as_str().map(str::to_owned),
        },
        stamp,
    );
    let mut bound = container["boundElements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    bound.push(json!({"type": "text", "id": label["id"]}));
    container["boundElements"] = json!(bound);
    fit(container, &mut label, stamp);
    label
}
