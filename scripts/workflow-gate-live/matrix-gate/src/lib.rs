//! wfgate acceptance matrix gate: the manual prompt selects the decision.
mod json;
wit_bindgen::generate!({ path: "/root/work/agent-wfgate/packages/app-wit/wit-gate", world: "gate" });
use exports::chariox::workflow_gate::evaluator::*;
use json::Json;
struct Gate;
export!(Gate);

fn plain(j: Json<'_>) -> &str {
    let raw = j.raw();
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') && !raw.contains('\\') {
        &raw[1..raw.len() - 1]
    } else {
        ""
    }
}
fn number(j: Json<'_>, default: u64) -> u64 {
    j.raw().parse().unwrap_or(default)
}
impl Guest for Gate {
    fn evaluate(input: Input, configuration: Configuration, state: State) -> Output {
        let i = Json::new(&input.entry_json);
        let cfg = Json::new(&configuration.config_json);
        let now = number(i.field("evaluation_at_ms"), 0);
        let accepted = number(i.field("accepted_at_ms"), 0);
        let deadline = number(i.field("deadline_ms"), 0);
        let entry = plain(i.field("entry_id"));
        let label = plain(cfg.field("label"));
        let step = number(cfg.field("step_ms"), 300);
        let nodes = i.field("allowed_entry_nodes").items();
        let node0 = nodes.first().map(|n| plain(*n)).unwrap_or("");
        let cmd = plain(i.field("trigger").field("prompt"));
        let mut parts = cmd.splitn(3, ':');
        let verb = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("");
        let arg2 = parts.next().unwrap_or("");
        let n: u64 = arg.parse().unwrap_or(0);
        let batch_phase = plain(i.field("phase")) == "batch_flush";
        let run = |payload: String, prompt: &str| {
            Decision::Run(vec![Route { node_id: node0.into(), payload_json: payload, prompt: Some(prompt.into()) }])
        };
        let ok_payload = format!("{{\"label\":\"{label}\",\"entry\":\"{entry}\",\"cmd\":\"{verb}\"}}");
        let decision = if batch_phase {
            run(i.field("batch").raw().into(), "Reply with the single word DIGEST.")
        } else {
            match verb {
                "run" => run(ok_payload, "Reply with the single word OK."),
                "drop" => Decision::Drop,
                // Wait n steps from acceptance, then run.
                "defer" => {
                    let due = accepted + n * step;
                    if now >= due { run(ok_payload, "Reply with the single word OK.") } else { Decision::DeferUntil((now + step).min(due).max(now + 1)) }
                }
                "forever" => Decision::DeferUntil(now + step),
                "late" => Decision::DeferUntil(deadline.saturating_sub(1000)),
                "past" => Decision::DeferUntil(now),
                "trap" => panic!("matrix trap"),
                "spin" => {
                    let mut x = 0u64;
                    loop {
                        x = core::hint::black_box(x.wrapping_add(1));
                    }
                }
                "badnode" => Decision::Run(vec![Route { node_id: "nope".into(), payload_json: "{}".into(), prompt: None }]),
                "batch" => {
                    let window = if n == 0 { 5 } else { n } * 1000;
                    let flush = (now / window + 1) * window;
                    Decision::Batch(BatchValue { key: if arg2.is_empty() { "k".into() } else { arg2.into() }, flush_at_ms: flush, payload_json: format!("{{\"entry\":\"{entry}\",\"label\":\"{label}\"}}") })
                }
                _ => Decision::Drop,
            }
        };
        let evaluated = number(Json::new(&state.workflow_json).field("evaluated"), 0) + 1;
        let last: String = verb.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        Output {
            decision,
            state: State { workflow_json: format!("{{\"evaluated\":{evaluated}}}"), binding_json: format!("{{\"last\":\"{last}\"}}") },
            reason: "matrix".into(),
            logs: vec![],
        }
    }
}
