wit_bindgen::generate!({ path: "../../../app-wit/wit-gate", world: "gate" });
use exports::chariox::workflow_gate::evaluator::*;
struct Gate;
impl Guest for Gate {
    fn evaluate(input: Input, configuration: Configuration, state: State) -> Output {
        let decision = match configuration.config_json.as_str() {
            "{\"mode\":\"loop\"}" => loop { core::hint::spin_loop(); },
            "{\"mode\":\"memory\"}" => { let data = vec![1u8; 32 * 1024 * 1024]; core::hint::black_box(data); Decision::Drop },
            "{\"mode\":\"huge-logs\"}" => return Output { decision: Decision::Drop, state, reason: String::new(), logs: vec![String::new(); 100_000] },
            "{\"mode\":\"bad-target\"}" => Decision::Run(vec![Route { node_id: "foreign".into(), payload_json: "{}".into(), prompt: None }]),
            "{\"mode\":\"duplicate-json\"}" => return Output { decision: Decision::Drop, state: State { workflow_json: "{\"seen\":1,\"seen\":2}".into(), binding_json: "{}".into() }, reason: String::new(), logs: vec![] },
            _ if input.entry_json.contains("\"severity\":\"high\"") => Decision::Run(vec![Route { node_id: "triage".into(), payload_json: "{\"severity\":\"high\"}".into(), prompt: Some("Assess the high advisory".into()) }]),
            _ => Decision::Drop,
        };
        Output { decision, state, reason: "evaluated".into(), logs: vec!["pure guest".into()] }
    }
}
export!(Gate);
