//! A one-shot migration instance with only the transactional state linker.
//! Snapshot/journal/activation authority remains with the kernel supervisor.
use crate::{
    host::{remote_error, Host},
    ipc::{error, wall_ms, Ipc},
    limits::{Limits, Stop},
    scope::Scope,
    worker::WorkerEngine,
};
use chariox_app_runtime::wire::RemoteError;
use std::time::Duration;
use wasmtime::{
    component::{Component, FutureReader, HasSelf, InstancePre, Linker},
    Store,
};

pub use crate::bindings::chariox::app::app_types as types;
mod bindings {
    wasmtime::component::bindgen!({
        path: "../app-wit/wit",
        world: "chariox:app/migration",
        require_store_data_send: true,
        with: {
            "chariox:app/state": crate::bindings::chariox::app::state,
            "chariox:app/app-types": crate::bindings::chariox::app::app_types,
        },
    });
}

pub struct MigrationWorker {
    engine: WorkerEngine,
    pre: InstancePre<Host>,
    limits: Limits,
}
fn check_scope(scope: &Scope, deadline: tokio::time::Instant) -> Result<(), RemoteError> {
    if let Some(reason) = scope.reason() {
        return Err(error(reason, "migration stopped"));
    }
    if tokio::time::Instant::now() >= deadline {
        scope.cancel("DEADLINE_EXCEEDED");
        return Err(error("DEADLINE_EXCEEDED", "migration deadline exceeded"));
    }
    Ok(())
}
impl MigrationWorker {
    pub fn new(
        engine: WorkerEngine,
        component: Component,
        limits: Limits,
    ) -> wasmtime::Result<Self> {
        limits.validate()?;
        let mut linker = Linker::new(engine.engine());
        crate::bindings::chariox::app::state::add_to_linker::<_, HasSelf<Host>>(
            &mut linker,
            |host| host,
        )?;
        let pre = linker.instantiate_pre(&component)?;
        Ok(Self {
            engine,
            pre,
            limits,
        })
    }
    pub async fn run(
        &self,
        ipc: Ipc,
        scope: Scope,
        from: u32,
        to: u32,
        mut context: types::InvocationContext,
    ) -> Result<(), RemoteError> {
        if from >= to {
            return Err(error(
                "INVALID_ARGUMENT",
                "migration must advance its schema",
            ));
        }
        let deadline = scope
            .deadline()
            .min(tokio::time::Instant::now() + Duration::from_millis(self.limits.call_ms));
        context.deadline_ms = wall_ms().saturating_add(
            deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_millis() as u64,
        );
        struct Finish(Scope);
        impl Drop for Finish {
            fn drop(&mut self) {
                self.0.finish();
            }
        }
        let _finish = Finish(scope.clone());
        check_scope(&scope, deadline)?;
        let host = Host::new(ipc, self.limits.memory_bytes, None)
            .map_err(|_| error("APP_UNAVAILABLE", "could not create migration host"))?;
        let attribution = host.attribution.clone();
        let mut store = Store::new(self.engine.engine(), host);
        store.limiter(|host| &mut host.memory);
        store.task_group_hook(attribution.clone());
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.slice_ms.div_ceil(5));
        let instance = tokio::time::timeout_at(deadline, self.pre.instantiate_async(&mut store))
            .await
            .map_err(|_| error("DEADLINE_EXCEEDED", "migration startup deadline exceeded"))?
            .map_err(|_| error("APP_TRAPPED", "migration startup trapped"))?;
        let migration = bindings::Migration::new(&mut store, &instance)
            .map_err(|_| error("APP_UNAVAILABLE", "invalid migration export"))?;
        check_scope(&scope, deadline)?;
        crate::limits::invocation_limits(&mut store, self.limits);
        let signal = scope.clone();
        let cancel = FutureReader::new(&mut store, async move {
            Ok::<_, wasmtime::Error>(signal.guest_cancelled().await.into())
        })
        .map_err(|_| error("APP_RESOURCE_LIMIT", "migration future table full"))?;
        attribution.starting(scope.clone());
        let running = store.run_concurrent(async |accessor| {
            migration
                .chariox_app_migration_handlers()
                .call_migrate(accessor, from, to, context, cancel)
                .await
        });
        tokio::pin!(running);
        let reason = tokio::select! {
            biased;
            result=&mut running => {
                // A guest may finish in the same poll that observes cancellation
                // or runs past its budget. Its result cannot authorize activation.
                check_scope(&scope, deadline)?;
                return result.and_then(std::convert::identity).map_err(|failure| {
                    if let Some(stop) = failure.downcast_ref::<Stop>() { error(stop.0,"migration stopped") }
                    else if failure.downcast_ref::<crate::limits::MemoryLimit>().is_some() { error("APP_MEMORY_LIMIT","migration memory limit exceeded") }
                    else { error("APP_TRAPPED","migration trapped") }
                })?.map_err(remote_error);
            },
            reason=scope.cancelled()=>reason,
            _=tokio::time::sleep_until(deadline)=>{scope.cancel("DEADLINE_EXCEEDED");"DEADLINE_EXCEEDED"},
        };
        let _ = tokio::time::timeout(
            Duration::from_millis(self.limits.cancel_grace_ms),
            &mut running,
        )
        .await;
        Err(error(reason, "migration stopped"))
    }
}
