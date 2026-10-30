//! Only supervisor-verified, descriptor-pinned migration components enter this
//! state-only phase. App registration cannot begin until every step commits.
use crate::{
    ipc::{wall_ms, Ipc},
    limits::Limits,
    migration::{types::InvocationContext, MigrationWorker},
    scope::Scope,
    worker::WorkerEngine,
};
use chariox_app_runtime::worker_process::wasm_config::WasmMigration;
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};

pub(super) async fn run(
    ipc: &Ipc,
    mut input: impl Read,
    installation: &str,
    steps: Vec<WasmMigration>,
) -> wasmtime::Result<()> {
    let deadline = tokio::time::Instant::now()
        + Duration::from_millis(chariox_app_runtime::worker_process::MIGRATION_TIMEOUT_MS);
    for step in steps {
        wasmtime::ensure!(
            step.bytes > 0 && step.bytes <= chariox_app_wit::MAX_COMPONENT_BYTES,
            "migration component exceeds limit"
        );
        let mut bytes = vec![0; step.bytes];
        input.read_exact(&mut bytes)?;
        wasmtime::ensure!(
            format!("sha256:{:x}", Sha256::digest(&bytes)) == step.digest,
            "migration component digest mismatch"
        );
        let engine = WorkerEngine::new()?;
        let component = engine.compile(&bytes, chariox_app_wit::MIGRATION_WORLD)?;
        let worker = MigrationWorker::new(engine, component, Limits::default())?;
        let scope = Scope::new(format!("migration-{}-{}", step.from, step.to), deadline);
        let context = InvocationContext {
            deadline_ms: wall_ms(),
            installation_id: Some(installation.into()),
            room_id: None,
            operation_id: None,
            agent_id: None,
            task_id: None,
            turn_id: None,
            actor: None,
        };
        if let Err(failure) = worker
            .run(ipc.clone(), scope, step.from, step.to, context)
            .await
        {
            // The kernel names this cause to the person; the process exit
            // alone would only say that the worker never registered.
            let report = Scope::new(
                format!("migration-failed-{}", step.to),
                tokio::time::Instant::now() + Duration::from_secs(2),
            );
            let _ = ipc
                .call(&report, "migration.failed", serde_json::json!({"code": failure.code}))
                .await;
            report.finish();
            return Err(wasmtime::Error::msg(format!("migration failed: {}", failure.code)));
        }
        let journal = Scope::new(format!("migration-step-{}", step.to), deadline);
        ipc.call(
            &journal,
            "migration.step",
            serde_json::json!({"to":step.to}),
        )
        .await
        .map_err(|e| wasmtime::Error::msg(format!("migration journal failed: {}", e.code)))?;
        journal.finish();
    }
    wasmtime::ensure!(input.read(&mut [0])? == 0, "unexpected migration bytes");
    Ok(())
}
