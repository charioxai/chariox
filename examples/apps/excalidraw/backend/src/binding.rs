//! Where an arrow meets the shapes it is bound to: the binding Excalidraw stores (`focus` and
//! `gap`, as `bindLinearElement` computes them for a straight arrow) and the path an arrow takes
//! between two shapes after they moved.

use crate::element::get;
use serde_json::Value;

type Point = (f64, f64);

fn center(shape: &Value) -> Point {
    (
        get(shape, "x") + get(shape, "width") / 2.0,
        get(shape, "y") + get(shape, "height") / 2.0,
    )
}

fn rotate(point: Point, around: Point, angle: f64) -> Point {
    let (dx, dy) = (point.0 - around.0, point.1 - around.1);
    let (sin, cos) = angle.sin_cos();
    (
        around.0 + dx * cos - dy * sin,
        around.1 + dx * sin + dy * cos,
    )
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Where the segments cross, if they do.
fn intersection(a: (Point, Point), b: (Point, Point)) -> Option<Point> {
    let (r, s) = (
        (a.1 .0 - a.0 .0, a.1 .1 - a.0 .1),
        (b.1 .0 - b.0 .0, b.1 .1 - b.0 .1),
    );
    let denominator = r.0 * s.1 - r.1 * s.0;
    if denominator == 0.0 {
        return None;
    }
    let (qp0, qp1) = (b.0 .0 - a.0 .0, b.0 .1 - a.0 .1);
    let t = (qp0 * s.1 - qp1 * s.0) / denominator;
    let u = (qp0 * r.1 - qp1 * r.0) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then(|| (a.0 .0 + t * r.0, a.0 .1 + t * r.1))
}

/// `determineFocusDistance`: where on the shape's focus image the line through `a` and `b` (`b`
/// nearer the shape) is aimed, as a ratio between -1 and 1; 0 is the centre.
pub fn focus(shape: &Value, a: Point, b: Point) -> f64 {
    let (x, y) = (get(shape, "x"), get(shape, "y"));
    let (width, height) = (get(shape, "width"), get(shape, "height"));
    let center = center(shape);
    if a == b {
        return 0.0;
    }
    let angle = get(shape, "angle");
    let (rotated_a, rotated_b) = (rotate(a, center, -angle), rotate(b, center, -angle));
    let cross = (rotated_b.0 - a.0) * (rotated_b.1 - center.1)
        - (rotated_b.1 - a.1) * (rotated_b.0 - center.0);
    let sign = if cross == 0.0 { 0.0 } else { -cross.signum() };
    let (dx, dy) = (rotated_b.0 - rotated_a.0, rotated_b.1 - rotated_a.1);
    let length = dx.hypot(dy);
    let reach = (width * 2.0).max(height * 2.0);
    let interceptor = (
        rotated_b,
        (
            rotated_b.0 + dx / length * reach,
            rotated_b.1 + dy / length * reach,
        ),
    );
    let diamond = shape["type"] == "diamond";
    let axes: [(Point, Point); 2] = if diamond {
        [
            ((x + width / 2.0, y), (x + width / 2.0, y + height)),
            ((x, y + height / 2.0), (x + width, y + height / 2.0)),
        ]
    } else {
        [
            ((x, y), (x + width, y + height)),
            ((x + width, y), (x, y + height)),
        ]
    };
    let interceptees: [(Point, Point); 2] = if diamond {
        [
            (
                (x + width / 2.0, y - height),
                (x + width / 2.0, y + height * 2.0),
            ),
            (
                (x - width, y + height / 2.0),
                (x + width * 2.0, y + height / 2.0),
            ),
        ]
    } else {
        [
            ((x - width, y - height), (x + width * 2.0, y + height * 2.0)),
            ((x + width * 2.0, y - height), (x - width, y + height * 2.0)),
        ]
    };
    let mut hits: Vec<Point> = interceptees
        .iter()
        .filter_map(|line| intersection(interceptor, *line))
        .collect();
    hits.sort_by(|g, h| {
        let (dg, dh) = (
            distance(*g, rotated_b).powi(2),
            distance(*h, rotated_b).powi(2),
        );
        dg.partial_cmp(&dh).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut ratios: Vec<f64> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let scale = if diamond {
                distance(axes[index].0, axes[index].1) / 2.0
            } else {
                width.hypot(height) / 2.0
            };
            sign * distance(center, *hit) / scale
        })
        .collect();
    ratios.sort_by(|g, h| {
        g.abs()
            .partial_cmp(&h.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ratios.first().copied().unwrap_or(0.0)
}

fn distance_to_segment(p: Point, (a, b): (Point, Point)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    };
    distance(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Distance from `p` to the outline of an ellipse with radii `rx`, `ry` about the origin, by
/// bisecting the parameter of the nearest point.
fn distance_to_ellipse(p: Point, rx: f64, ry: f64) -> f64 {
    let (px, py) = (p.0.abs(), p.1.abs());
    let at = |t: f64| (rx * t.cos(), ry * t.sin());
    let steps = 360;
    let mut best = (f64::MAX, 0.0);
    for i in 0..=steps {
        let t = std::f64::consts::FRAC_PI_2 * i as f64 / steps as f64;
        let d = distance((px, py), at(t));
        if d < best.0 {
            best = (d, t);
        }
    }
    let width = std::f64::consts::FRAC_PI_2 / steps as f64;
    let (mut low, mut high) = (
        (best.1 - width).max(0.0),
        (best.1 + width).min(std::f64::consts::FRAC_PI_2),
    );
    for _ in 0..40 {
        let (m1, m2) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        if distance((px, py), at(m1)) < distance((px, py), at(m2)) {
            high = m2;
        } else {
            low = m1;
        }
    }
    distance((px, py), at((low + high) / 2.0))
}

/// `distanceToBindableElement`, with the corners of a rounded rectangle or diamond taken sharp.
fn outline_distance(shape: &Value, p: Point) -> f64 {
    let (x, y) = (get(shape, "x"), get(shape, "y"));
    let (width, height) = (get(shape, "width"), get(shape, "height"));
    let c = center(shape);
    let p = rotate(p, c, -get(shape, "angle"));
    match shape["type"].as_str() {
        Some("ellipse") => distance_to_ellipse((p.0 - c.0, p.1 - c.1), width / 2.0, height / 2.0),
        Some("diamond") => {
            let corners = [
                (x + width / 2.0, y),
                (x + width, y + height / 2.0),
                (x + width / 2.0, y + height),
                (x, y + height / 2.0),
            ];
            (0..4)
                .map(|i| distance_to_segment(p, (corners[i], corners[(i + 1) % 4])))
                .fold(f64::MAX, f64::min)
        }
        _ => {
            let corners = [
                (x, y),
                (x + width, y),
                (x + width, y + height),
                (x, y + height),
            ];
            (0..4)
                .map(|i| distance_to_segment(p, (corners[i], corners[(i + 1) % 4])))
                .fold(f64::MAX, f64::min)
        }
    }
}

/// How far from a shape an arrow's end may be and still count as touching it.
fn max_gap(shape: &Value) -> f64 {
    let ratio = if shape["type"] == "diamond" {
        1.0 / 2f64.sqrt()
    } else {
        1.0
    };
    let smaller = ratio * get(shape, "width").min(get(shape, "height"));
    16.0f64.max((0.25 * smaller).min(32.0)).max(10.0 + 4.0)
}

/// The binding of the arrow's end at `edge` (the other point of the arrow is `adjacent`) to `shape`.
pub fn bind(shape: &Value, adjacent: Point, edge: Point) -> (f64, f64) {
    let gap = outline_distance(shape, edge).max(1.0);
    let gap = if gap > max_gap(shape) {
        10.0 + 4.0
    } else {
        gap
    };
    (focus(shape, adjacent, edge), gap)
}

/// Where a line from `shape`'s centre towards `towards` leaves the shape, with five units of air.
pub fn boundary(shape: &Value, towards: Point) -> Point {
    let c = center(shape);
    let angle = get(shape, "angle");
    let (cos, sin) = (angle.cos(), angle.sin());
    let (vx, vy) = (towards.0 - c.0, towards.1 - c.1);
    let (dx, dy) = (vx * cos + vy * sin, -vx * sin + vy * cos);
    let (rx, ry) = (get(shape, "width") / 2.0, get(shape, "height") / 2.0);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return c;
    }
    let scale = match shape["type"].as_str() {
        Some("ellipse") => 1.0 / (dx / rx).hypot(dy / ry),
        Some("diamond") => 1.0 / (dx.abs() / rx + dy.abs() / ry),
        _ => (rx
            / if dx == 0.0 {
                f64::MIN_POSITIVE
            } else {
                dx.abs()
            })
        .min(
            ry / if dy == 0.0 {
                f64::MIN_POSITIVE
            } else {
                dy.abs()
            },
        ),
    };
    (
        c.0 + vx * (scale + 5.0 / length),
        c.1 + vy * (scale + 5.0 / length),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shape(kind: &str, x: f64, y: f64, width: f64, height: f64) -> Value {
        json!({"type": kind, "x": x, "y": y, "width": width, "height": height, "angle": 0})
    }

    #[test]
    fn a_line_through_the_centre_has_no_focus_and_the_gap_is_at_least_one() {
        let rectangle = shape("rectangle", 0.0, 0.0, 100.0, 50.0);
        assert_eq!(focus(&rectangle, (300.0, 25.0), (100.0, 25.0)), 0.0);
        let (focus, gap) = bind(&rectangle, (300.0, 25.0), (100.0, 25.0));
        assert_eq!((focus, gap), (0.0, 1.0));
    }

    #[test]
    fn a_line_aimed_off_centre_has_a_signed_focus_that_flips_with_the_side() {
        let rectangle = shape("rectangle", 0.0, 0.0, 100.0, 100.0);
        let above = focus(&rectangle, (300.0, 0.0), (100.0, 20.0));
        let below = focus(&rectangle, (300.0, 100.0), (100.0, 80.0));
        assert!(
            above != 0.0 && above.signum() == -below.signum(),
            "{above} {below}"
        );
        assert!(above.abs() < 1.0);
    }

    #[test]
    fn distances_measure_to_the_outline_of_each_shape() {
        let rectangle = shape("rectangle", 0.0, 0.0, 100.0, 50.0);
        assert!((outline_distance(&rectangle, (110.0, 25.0)) - 10.0).abs() < 1e-9);
        let ellipse = shape("ellipse", 0.0, 0.0, 100.0, 50.0);
        assert!(outline_distance(&ellipse, (100.0, 25.0)) < 1e-6);
        assert!((outline_distance(&ellipse, (160.0, 25.0)) - 60.0).abs() < 1e-3);
        let diamond = shape("diamond", 0.0, 0.0, 100.0, 100.0);
        assert!(outline_distance(&diamond, (100.0, 50.0)) < 1e-9);
    }

    #[test]
    fn the_boundary_is_where_the_line_to_the_other_shape_leaves_the_edge() {
        let rectangle = shape("rectangle", 0.0, 0.0, 100.0, 50.0);
        let (x, y) = boundary(&rectangle, (300.0, 25.0));
        assert!(
            (x - 105.0).abs() < 1e-6 && (y - 25.0).abs() < 1e-6,
            "{x} {y}"
        );
        let ellipse = shape("ellipse", 0.0, 0.0, 100.0, 50.0);
        assert!((boundary(&ellipse, (300.0, 25.0)).0 - 105.0).abs() < 1e-6);
        assert_eq!(boundary(&rectangle, (50.0, 25.0)), (50.0, 25.0));
    }
}
