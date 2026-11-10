//! One evaluation per contained process. This path has no App host, linker,
//! retry policy, database, broker, WASI or installation directories.
mod bindings;
mod bounded;
use chariox_app_runtime::workflow_gate as contract;
use contract::{Evaluation, Failure, Output, Report};
use std::{io::Write, time::Instant};
use wasmtime::{component::Linker, Store};

pub(crate) fn serve() -> wasmtime::Result<()> {
    unsafe {
        libc::close(8);
    }
    let bytes = crate::supervised::read(6, chariox_app_wit::gate::MAX_COMPONENT_BYTES)?;
    let launch_bytes = crate::supervised::read(5, contract::INPUT_BYTES + 1024)?;
    let value = contract::parse_json(&launch_bytes, contract::INPUT_BYTES + 1024)?;
    let launch: contract::Launch = serde_json::from_value(value)?;
    launch.evaluation.validate()?;
    wasmtime::ensure!(
        contract::sha256(&bytes) == launch.module_sha256,
        "gate module digest mismatch"
    );
    // Mandatory before readiness, admission, compilation and epoch threads.
    if let Err(error) = crate::containment::enter_gate() {
        if let Ok(mut stream) = crate::cli::inherited_stream(4) {
            let _ = stream.write_all(b"CXAWC001");
        }
        return Err(error.into());
    }
    crate::cli::ready_gate(&launch.generation, &launch.entry_id, &launch.module_sha256)?;
    let report = evaluate(&bytes, &launch.evaluation, launch.validate_only);
    let response = contract::Response {
        generation: launch.generation,
        entry_id: launch.entry_id,
        module_sha256: launch.module_sha256,
        report,
    };
    let response = serde_json::to_vec(&response)?;
    wasmtime::ensure!(
        response.len() <= contract::OUTPUT_BYTES + 1024,
        "gate response exceeds wire ceiling"
    );
    let mut stream = crate::cli::inherited_stream(3)?;
    stream.write_all(&(response.len() as u32).to_be_bytes())?;
    stream.write_all(&response)?;
    // No second request, unsolicited event or callback is possible.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

fn evaluate(bytes: &[u8], evaluation: &Evaluation, validate_only: bool) -> Report {
    let started = Instant::now();
    let mut fuel_consumed = 0;
    let outcome = (|| -> wasmtime::Result<Output> {
        let engine = crate::worker::WorkerEngine::gate()?;
        let component = engine.compile(bytes, chariox_app_wit::gate::WORLD)?;
        if validate_only {
            return Ok(Output {
                decision: contract::Decision::Drop,
                state: evaluation.state.clone(),
                reason: "component validated".into(),
                logs: vec![],
            });
        }
        let linker = Linker::<crate::limits::Memory>::new(engine.engine());
        let mut store = Store::new(engine.engine(), crate::limits::Memory::gate());
        store.limiter(|memory| memory);
        store.set_fuel(contract::FUEL)?;
        store.set_epoch_deadline(contract::EXECUTION_MS / 5);
        let result = (|| {
            let instance = linker.instantiate(&mut store, &component)?;
            let (_, interface) = instance
                .get_export(&mut store, None, "chariox:workflow-gate/evaluator@1.0.0")
                .ok_or_else(|| wasmtime::Error::msg("gate evaluator export missing"))?;
            let (_, function) = instance
                .get_export(&mut store, Some(&interface), "evaluate")
                .ok_or_else(|| wasmtime::Error::msg("gate evaluate missing"))?;
            let function = instance.get_typed_func::<(
                bindings::Input,
                bindings::Configuration,
                bindings::StateInput,
            ), (bindings::Output,)>(&mut store, &function)?;
            let config = &evaluation.configuration;
            bounded::begin();
            let (raw,) = function.call(
                &mut store,
                (
                    bindings::Input {
                        entry_json: String::from_utf8(contract::canonical_json(
                            &serde_json::to_value(&evaluation.input)?,
                            contract::INPUT_BYTES,
                        )?)?,
                    },
                    bindings::Configuration {
                        workflow_revision: config.workflow_revision.clone(),
                        config_sha256: config.config_sha256.clone(),
                        config_json: String::from_utf8(contract::canonical_json(
                            &config.config.clone().into(),
                            contract::CONFIG_BYTES,
                        )?)?,
                    },
                    bindings::StateInput {
                        workflow_json: serde_json::to_string(&evaluation.state.workflow)?,
                        binding_json: serde_json::to_string(&evaluation.state.binding)?,
                    },
                ),
            )?;
            let output = lift(raw)?;
            // Wasmtime 50 includes post-return in call under the same budget.
            output.validate(evaluation)?;
            Ok(output)
        })();
        fuel_consumed = contract::FUEL.saturating_sub(store.get_fuel()?);
        result
    })()
    .map_err(classify);
    Report {
        outcome,
        fuel_consumed,
        execution_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        profile: crate::engine::GATE_ENGINE_CONFIG.into(),
    }
}
fn classify(error: wasmtime::Error) -> Failure {
    if let Some(failure) = error.downcast_ref::<Failure>() {
        return *failure;
    }
    if error.downcast_ref::<crate::limits::MemoryLimit>().is_some() {
        return Failure::GateResourceLimit;
    }
    match error.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::OutOfFuel) => Failure::GateFuelExhausted,
        Some(wasmtime::Trap::Interrupt) => Failure::GateTimeout,
        _ => Failure::GateTrapped,
    }
}
fn lift(raw: bindings::Output) -> wasmtime::Result<Output> {
    fn json<const N: usize>(text: bounded::Text<N>) -> wasmtime::Result<serde_json::Value> {
        Ok(contract::parse_json(text.0.as_bytes(), N).map_err(|_| Failure::GateInvalidOutput)?)
    }
    let workflow = json(raw.state.workflow_json)?;
    let binding = json(raw.state.binding_json)?;
    let state = contract::State {
        workflow: workflow
            .as_object()
            .ok_or(Failure::GateInvalidOutput)?
            .clone(),
        binding: binding
            .as_object()
            .ok_or(Failure::GateInvalidOutput)?
            .clone(),
    };
    let decision = match raw.decision {
        bindings::Decision::Drop => contract::Decision::Drop,
        bindings::Decision::DeferUntil(at_ms) => contract::Decision::DeferUntil { at_ms },
        bindings::Decision::Batch(batch) => contract::Decision::Batch {
            key: batch.key.0,
            flush_at_ms: batch.flush_at_ms,
            payload: json(batch.payload_json)?,
        },
        bindings::Decision::Run(routes) => {
            let targets = routes
                .0
                .into_iter()
                .map(|route| {
                    Ok(contract::Route {
                        node_id: route.node_id.0,
                        payload: json(route.payload_json)?,
                        prompt: route.prompt.map(|prompt| prompt.0),
                    })
                })
                .collect::<wasmtime::Result<Vec<_>>>()?;
            contract::Decision::Run { targets }
        }
    };
    Ok(Output {
        decision,
        state,
        reason: raw.reason.0,
        logs: raw.logs.0.into_iter().map(|text| text.0).collect(),
    })
}
