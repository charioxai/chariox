use chariox_app_runtime::wire::{decode, Sender};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const CORPUS: &str = include_str!("../../app-sdk/test/wire-vectors.json");

#[test]
fn rust_and_sdk_enforce_the_same_wire_corpus() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap();
    let generation = corpus["generation"].as_str().unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let sender = if case["sender"] == "worker" {
            Sender::Worker
        } else {
            Sender::Supervisor
        };
        let result = decode(
            &serde_json::to_vec(&case["message"]).unwrap(),
            generation,
            sender,
        );
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["name"]
        );
    }
    for case in corpus["rawCases"].as_array().unwrap() {
        let sender = if case["sender"] == "worker" {
            Sender::Worker
        } else {
            Sender::Supervisor
        };
        let result = decode(
            case["json"].as_str().unwrap().as_bytes(),
            generation,
            sender,
        );
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["name"]
        );
    }
}

#[test]
fn rust_encoded_messages_round_trip_through_the_real_node_sdk() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap();
    let generation = corpus["generation"].as_str().unwrap();
    let valid: Vec<_> = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["valid"] == true)
        .collect();
    let mut input = Vec::new();
    for case in &valid {
        let sender = if case["sender"] == "worker" {
            Sender::Worker
        } else {
            Sender::Supervisor
        };
        let message = decode(
            &serde_json::to_vec(&case["message"]).unwrap(),
            generation,
            sender,
        )
        .unwrap();
        let bytes = serde_json::to_vec(&message).unwrap();
        input.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        input.extend_from_slice(&bytes);
    }
    assert!(input.len() < 4096, "keep fixture below pipe capacity");
    let script = r#"
        import { FrameDecoder, encodeFrame } from './src/internal.js';
        const decoder = new FrameDecoder(message => process.stdout.write(encodeFrame(message)));
        process.stdin.on('data', chunk => decoder.push(chunk));
        process.stdin.on('end', () => decoder.end());
    "#;
    let sdk_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app-sdk");
    let mut child = Command::new("node")
        .args(["--input-type=module", "--eval", script])
        .current_dir(sdk_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for the App SDK interoperability gate");
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > std::time::Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Node interoperability fixture timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut framed = output.stdout.as_slice();
    let mut count = 0;
    while !framed.is_empty() {
        let length = u32::from_be_bytes(framed[..4].try_into().unwrap()) as usize;
        let decoded = decode(&framed[4..length + 4], generation, Sender::Supervisor).unwrap();
        let expected = &valid[count]["message"];
        assert_eq!(serde_json::to_value(decoded).unwrap(), *expected);
        count += 1;
        framed = &framed[length + 4..];
    }
    assert_eq!(
        count,
        corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["valid"] == true)
            .count()
    );
}
