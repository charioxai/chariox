use super::*;
use std::collections::BTreeSet;

impl State {
    pub fn validate(&self) -> Result<(), Failure> {
        for state in [&self.workflow, &self.binding] {
            canonical_json(&Value::Object(state.clone()), STATE_BYTES)?;
        }
        Ok(())
    }
}
impl Evaluation {
    pub fn validate(&self) -> Result<(), Failure> {
        let input = &self.input;
        if input.abi != ABI
            || input.entry_id.is_empty()
            || input.endpoint_id.is_empty()
            || input.binding_id.is_empty()
            || self.configuration.workflow_revision.is_empty()
            || input.allowed_entry_nodes.is_empty()
            || input.allowed_entry_nodes.len() > MAX_TARGETS
            || input
                .allowed_entry_nodes
                .iter()
                .any(|id| id.is_empty() || id.len() > 128)
            || input
                .allowed_entry_nodes
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != input.allowed_entry_nodes.len()
            || input.accepted_at_ms > input.evaluation_at_ms
            || input.evaluation_at_ms >= input.deadline_ms
            || input.deadline_ms > MAX_SAFE_INTEGER
            || !matches!(
                (&input.trigger, &input.batch, input.phase),
                (Some(_), None, Phase::Trigger | Phase::Resume)
                    | (None, Some(_), Phase::BatchFlush | Phase::Resume)
            )
        {
            return Err(Failure::GateInvalidInput);
        }
        let config = canonical_json(
            &Value::Object(self.configuration.config.clone()),
            CONFIG_BYTES,
        )?;
        if sha256(&config) != self.configuration.config_sha256 {
            return Err(Failure::GateInvalidInput);
        }
        self.state.validate()?;
        let value = serde_json::to_value(self).map_err(|_| Failure::GateInvalidInput)?;
        canonical_json(&value, INPUT_BYTES).map_err(|error| {
            if error == Failure::GateLimit {
                Failure::GateInputLimit
            } else {
                error
            }
        })?;
        Ok(())
    }
}
impl Output {
    pub fn validate(&self, evaluation: &Evaluation) -> Result<(), Failure> {
        self.state
            .validate()
            .map_err(|_| Failure::GateInvalidOutput)?;
        if self.reason.len() > 512
            || self.logs.len() > 32
            || self.logs.iter().any(|log| log.len() > 256)
        {
            return Err(Failure::GateLimit);
        }
        let input = &evaluation.input;
        let valid_time = |time: u64| -> Result<(), Failure> {
            if time <= input.evaluation_at_ms || time >= input.deadline_ms {
                return Err(Failure::GateInvalidTime);
            }
            Ok(())
        };
        match &self.decision {
            Decision::Run { targets } => {
                if targets.is_empty() || targets.len() > MAX_TARGETS {
                    return Err(Failure::GateInvalidTarget);
                }
                let mut next_position = 0;
                let mut bytes = 0;
                for route in targets {
                    let Some(index) = input.allowed_entry_nodes[next_position..]
                        .iter()
                        .position(|id| id == &route.node_id)
                    else {
                        return Err(Failure::GateInvalidTarget);
                    };
                    next_position += index + 1;
                    bytes += canonical_json(&route.payload, CONTEXT_BYTES)?.len()
                        + route.prompt.as_ref().map_or(0, String::len);
                }
                if bytes > CONTEXT_BYTES {
                    return Err(Failure::GateLimit);
                }
            }
            Decision::Drop => {}
            Decision::DeferUntil { at_ms } => valid_time(*at_ms)?,
            Decision::Batch {
                key,
                flush_at_ms,
                payload,
            } => {
                if input.batch.is_some() {
                    return Err(Failure::GateInvalidOutput);
                }
                if key.is_empty() || key.len() > 128 {
                    return Err(Failure::GateLimit);
                }
                valid_time(*flush_at_ms)?;
                canonical_json(payload, CONTEXT_BYTES)?;
            }
        }
        canonical_json(
            &serde_json::to_value(self).map_err(|_| Failure::GateInvalidOutput)?,
            OUTPUT_BYTES,
        )
        .map_err(|_| Failure::GateInvalidOutput)?;
        Ok(())
    }
}
