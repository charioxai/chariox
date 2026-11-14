//! The actions against what the page produced for the same requests: `tests/golden.json` holds,
//! for each request, the elements Excalidraw's own converter and layout returned
//! (`scripts/golden.mjs` makes it from the real library in Chrome).

use crate::{actions, stamp::Stamp};
use serde_json::{json, Value};

const GOLDEN: &str = include_str!("../tests/golden.json");

/// Fields that differ between two runs of the same request.
const VOLATILE: [&str; 4] = ["seed", "versionNonce", "updated", "index"];

fn normalized(element: &Value) -> Value {
    let mut element = element.clone();
    for key in VOLATILE {
        element.as_object_mut().unwrap().remove(key);
    }
    element
}

fn close(a: &Value, b: &Value, path: &str, problems: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() > 0.02 {
                problems.push(format!("{path}: {x} != {y}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for key in x
                .keys()
                .chain(y.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                close(
                    x.get(key).unwrap_or(&Value::Null),
                    y.get(key).unwrap_or(&Value::Null),
                    &format!("{path}.{key}"),
                    problems,
                );
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                close(p, q, &format!("{path}[{i}]"), problems);
            }
        }
        _ if a == b => {}
        _ => problems.push(format!("{path}: {a} != {b}")),
    }
}

/// Rename the IDs of the elements we made, by position, to the IDs the page's elements have: the
/// page made random IDs for bound text and the arrow of a connection.
fn adopt_ids(scene: &mut Value, golden: &[Value]) {
    let elements = scene["elements"].as_array_mut().unwrap();
    let renames: Vec<(String, String)> = elements
        .iter()
        .zip(golden)
        .filter_map(|(ours, theirs)| {
            Some((
                ours["id"].as_str()?.to_owned(),
                theirs["id"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let text = scene.to_string();
    let mut text = text;
    for (from, to) in renames.iter().filter(|(from, to)| from != to) {
        text = text.replace(&format!("\"{from}\""), &format!("\"{to}\""));
    }
    *scene = serde_json::from_str(&text).unwrap();
}

fn live_elements(scene: &Value) -> Vec<Value> {
    scene["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["isDeleted"] != true)
        .cloned()
        .collect()
}

#[test]
fn every_action_makes_what_the_page_made() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let mut failures = Vec::new();
    let mut steps = 0;
    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut scene = json!({"type": "excalidraw", "version": 2, "elements": [], "appState": {}, "files": {}});
        for (n, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            steps += 1;
            let action = step["action"].as_str().unwrap();
            let mut stamp = Stamp::new(0x9E37_79B9_7F4A_7C15 + n as u64, 1_700_000_000_000);
            let before = scene.clone();
            let result = actions::edit(action, &mut scene, &step["input"], &mut stamp);
            let expected = step["elements"].as_array().unwrap();
            match (step.get("error").and_then(Value::as_str), &result) {
                (Some(message), Err(e)) => {
                    if e.message != message {
                        failures.push(format!("{name}#{n}: error {:?} != {message:?}", e.message));
                    }
                    scene = before;
                    continue;
                }
                (Some(message), Ok(())) => {
                    failures.push(format!("{name}#{n}: expected the error {message:?}"));
                    continue;
                }
                (None, Err(e)) => {
                    failures.push(format!("{name}#{n}: failed with {}", e.message));
                    scene = before;
                    continue;
                }
                (None, Ok(())) => {}
            }
            adopt_ids(&mut scene, expected);
            let ours = live_elements(&scene);
            if ours.len() != expected.len() {
                failures.push(format!(
                    "{name}#{n}: {} elements, the page made {}",
                    ours.len(),
                    expected.len()
                ));
                continue;
            }
            for (ours, theirs) in ours.iter().zip(expected) {
                let mut problems = Vec::new();
                close(
                    &normalized(ours),
                    &normalized(theirs),
                    theirs["id"].as_str().unwrap(),
                    &mut problems,
                );
                failures.extend(problems.into_iter().map(|p| format!("{name}#{n}: {p}")));
            }
        }
    }
    assert!(
        steps > 70,
        "the golden file holds the cases ({steps} steps)"
    );
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
