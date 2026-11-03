//! Real contained worker drill. Build the Rust guest with the locked recipe in
//! tests/gate-fixture, encode with chariox-gate-component, then set
//! CHARIOX_GATE_COMPONENT to its portable output. No provider or App broker.
use chariox_app_runtime::{
    worker_process::{gate::evaluate_blocking, wasm::WasmRuntime},
    workflow_gate::*,
};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn pure_gate_is_contained_metered_and_validated_before_release() {
    let component =
        fs::read(std::env::var_os("CHARIOX_GATE_COMPONENT").expect("build the gate fixture first"))
            .unwrap();
    let root = std::env::temp_dir().join(format!("gate-drill-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let worker = root.join("chariox-app-wasm-worker");
    let source = std::env::var_os("CHARIOX_GATE_WORKER")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_chariox-app-wasm-worker").into());
    fs::copy(source, &worker).unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o555)).unwrap();
    let base = Evaluation {
        input: Input {
            abi: ABI.into(),
            entry_id: "entry-1".into(),
            endpoint_id: "entry".into(),
            allowed_entry_nodes: vec!["triage".into()],
            binding_id: "binding-1".into(),
            phase: Phase::Trigger,
            accepted_at_ms: 1,
            evaluation_at_ms: 2,
            deadline_ms: 1000,
            trigger: Some(Trigger {
                kind: TriggerKind::AppEvent,
                occurrence_id: "advisory-1".into(),
                occurred_at_ms: Some(1),
                data: Some(json!({"severity":"high"}).as_object().unwrap().clone()),
                data_schema: None,
                prompt: Some("Do not retain this original prompt".into()),
                artifacts: vec![],
            }),
            batch: None,
        },
        configuration: Configuration::new("revision-1".into(), Default::default()).unwrap(),
        state: State::default(),
    };
    for (index, mode) in [
        "run",
        "loop",
        "memory",
        "huge-logs",
        "bad-target",
        "duplicate-json",
    ]
    .iter()
    .enumerate()
    {
        let mut evaluation = base.clone();
        evaluation.configuration = Configuration::new(
            "revision-1".into(),
            json!({"mode":mode}).as_object().unwrap().clone(),
        )
        .unwrap();
        let launch = Launch {
            validate_only: false,
            generation: (index + 1).to_string(),
            entry_id: evaluation.input.entry_id.clone(),
            module_sha256: sha256(&component),
            evaluation,
        };
        let report = evaluate_blocking(
            WasmRuntime::open(&worker).unwrap(),
            &launch,
            &component,
            &root,
        )
        .unwrap();
        assert_eq!(report.profile, PROFILE);
        assert!(report.fuel_consumed <= FUEL);
        match *mode {
            "run" => {
                let output = report.outcome.unwrap();
                let Decision::Run { targets } = output.decision else {
                    panic!("expected selected route");
                };
                assert_eq!(targets.len(), 1);
                assert_eq!(targets[0].node_id, "triage");
                assert_eq!(
                    targets[0].prompt.as_deref(),
                    Some("Assess the high advisory")
                );
                assert!(!serde_json::to_string(&targets)
                    .unwrap()
                    .contains("original prompt"));
            }
            "loop" => assert_eq!(report.outcome.unwrap_err(), Failure::GateFuelExhausted),
            "memory" => assert_eq!(report.outcome.unwrap_err(), Failure::GateResourceLimit),
            "huge-logs" => assert_eq!(report.outcome.unwrap_err(), Failure::GateLimit),
            "bad-target" => assert_eq!(report.outcome.unwrap_err(), Failure::GateInvalidTarget),
            "duplicate-json" => assert_eq!(report.outcome.unwrap_err(), Failure::GateInvalidOutput),
            _ => unreachable!(),
        }
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn advisory_component_selects_triage_and_one_summary_flush() {
    let component = fs::read(
        std::env::var_os("CHARIOX_ADVISORY_GATE_COMPONENT")
            .expect("build examples/workflow-gates/advisory first"),
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("advisory-gate-drill-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let worker = root.join("chariox-app-wasm-worker");
    let source = std::env::var_os("CHARIOX_GATE_WORKER")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_chariox-app-wasm-worker").into());
    fs::copy(source, &worker).unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o555)).unwrap();
    let mut evaluation = Evaluation {
        input: Input {
            abi: ABI.into(),
            entry_id: "advisory-entry".into(),
            endpoint_id: "entry".into(),
            allowed_entry_nodes: vec!["triage".into(), "summary".into()],
            binding_id: "upwatch".into(),
            phase: Phase::Trigger,
            accepted_at_ms: 1,
            evaluation_at_ms: 2,
            deadline_ms: 10_000,
            trigger: Some(Trigger {
                kind: TriggerKind::AppEvent,
                occurrence_id: "advisory".into(),
                occurred_at_ms: Some(1),
                data: Some(json!({"severity":"high"}).as_object().unwrap().clone()),
                data_schema: None,
                prompt: None,
                artifacts: vec![],
            }),
            batch: None,
        },
        configuration: Configuration::new(
            "1".into(),
            json!({"batch_window_ms":1000}).as_object().unwrap().clone(),
        )
        .unwrap(),
        state: State::default(),
    };
    for step in 0..3 {
        if step == 1 {
            evaluation.input.trigger.as_mut().unwrap().data =
                Some(json!({"severity":"low"}).as_object().unwrap().clone());
        }
        if step == 2 {
            evaluation.input.trigger = None;
            evaluation.input.phase = Phase::BatchFlush;
            evaluation.input.evaluation_at_ms = 1000;
            evaluation.input.batch = Some(Batch {
                key: "public-advisories".into(),
                flush_at_ms: 1000,
                items: vec![BatchItem {
                    entry_id: "low-entry".into(),
                    payload: json!({"severity":"low"}),
                }],
            });
        }
        let report = evaluate_blocking(
            WasmRuntime::open(&worker).unwrap(),
            &Launch {
                validate_only: false,
                generation: format!("advisory-{step}"),
                entry_id: evaluation.input.entry_id.clone(),
                module_sha256: sha256(&component),
                evaluation: evaluation.clone(),
            },
            &component,
            &root,
        )
        .unwrap();
        assert!(report.fuel_consumed <= FUEL);
        let output = report.outcome.unwrap();
        match &output.decision {
            Decision::Run { targets } => assert_eq!(
                targets
                    .iter()
                    .map(|r| r.node_id.as_str())
                    .collect::<Vec<_>>(),
                vec![if step == 0 { "triage" } else { "summary" }]
            ),
            Decision::Batch {
                key, flush_at_ms, ..
            } => {
                assert_eq!(step, 1);
                assert_eq!(key, "public-advisories");
                assert_eq!(*flush_at_ms, 1000);
            }
            decision => panic!("unexpected advisory decision {decision:?}"),
        }
        assert_eq!(
            output.state.workflow.get("evaluated"),
            Some(&json!(step + 1))
        );
        evaluation.state = output.state;
    }
    fs::remove_dir_all(root).unwrap();
}
