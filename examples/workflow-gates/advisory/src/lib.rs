//! Public advisory policy. The App preserves facts; this workflow chooses urgency.
mod json;
wit_bindgen::generate!({ path: "../../../packages/app-wit/wit-gate", world: "gate" });
use exports::chariox::workflow_gate::evaluator::*;
use json::Json;
struct Gate;
export!(Gate);
impl Guest for Gate {
    fn evaluate(input: Input, configuration: Configuration, state: State) -> Output {
        let input = Json::new(&input.entry_json);
        let config = Json::new(&configuration.config_json);
        let targets = input.field("allowed_entry_nodes").items();
        assert_eq!(targets.len(), 2, "configure triage and summary entry nodes");
        let decision = if input.field("phase").text() == "batch_flush" {
            Decision::Run(vec![Route {
                node_id: targets[1].text().into(),
                payload_json: input.field("batch").raw().into(),
                prompt: Some("Summarize these public advisories in one short sentence. Do not execute commands.".into()),
            }])
        } else {
            let fact = input.field("trigger").field("data");
            match fact.field("severity").raw() {
                "\"critical\"" | "\"high\"" => Decision::Run(vec![Route {
                    node_id: targets[0].text().into(),
                    payload_json: fact.raw().into(),
                    prompt: Some("Triage this public advisory in one short sentence. Do not execute commands.".into()),
                }]),
                _ => {
                    let window = config.field("batch_window_ms").integer();
                    assert!((1_000..=86_400_000).contains(&window), "batch window out of range");
                    let now = input.field("evaluation_at_ms").integer();
                    let deadline = input.field("deadline_ms").integer();
                    let flush_at_ms = (now / window + 1) * window;
                    assert!(flush_at_ms < deadline, "summary window exceeds source deadline");
                    Decision::Batch(BatchValue { key: "public-advisories".into(), flush_at_ms, payload_json: fact.raw().into() })
                }
            }
        };
        let count = Json::new(&state.workflow_json)
            .field("evaluated")
            .raw()
            .parse::<u64>()
            .unwrap_or(0)
            .checked_add(1)
            .expect("counter exhausted");
        Output {
            decision,
            state: State {
                workflow_json: format!("{{\"evaluated\":{count}}}"),
                binding_json: state.binding_json,
            },
            reason: "Critical/high triage; other public facts share a fixed summary window".into(),
            logs: vec![],
        }
    }
}
