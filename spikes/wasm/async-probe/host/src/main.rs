//! Runs N concurrent `run(1)` calls on ONE instance; each waits 50 ms on the
//! host. Concurrent: ~50 ms total. Serialized: ~N x 50 ms.
use std::time::{Duration, Instant};
use wasmtime::component::{Accessor, Component, HasSelf, Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime::{Config, Engine, Store};

wasmtime::component::bindgen!({ world: "probe", path: "../wit" });

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl spike::probe::kernel::Host for Host {}
impl<T: Send> spike::probe::kernel::HostWithStore<T> for HasSelf<Host> {
    async fn get(_accessor: &Accessor<T, Self>, key: String) -> String {
        tokio::time::sleep(Duration::from_millis(50)).await;
        key
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> wasmtime::Result<()> {
    let path = std::env::args().nth(1).expect("component path");
    let calls: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(8);
    let mut config = Config::new();
    // SPIKE_CM_ASYNC=default leaves wasmtime's own default in place.
    if std::env::var("SPIKE_CM_ASYNC").as_deref() != Ok("default") {
        config.wasm_component_model_async(true);
    }
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &path)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    Probe::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
    let mut store = Store::new(&engine, Host { wasi: WasiCtx::builder().build(), table: ResourceTable::new() });
    let probe = Probe::instantiate_async(&mut store, &component, &linker).await?;
    let started = Instant::now();
    let results = store.run_concurrent(async |accessor| {
        let mut pending = Vec::new();
        for _ in 0..calls {
            pending.push(probe.call_run(accessor, 1));
        }
        futures::future::join_all(pending).await
    }).await?;
    let ok = results.iter().filter(|r| r.is_ok()).count();
    println!("{{\"calls\":{calls},\"ok\":{ok},\"totalMs\":{:.1},\"serializedWouldBeMs\":{}}}", started.elapsed().as_secs_f64() * 1e3, calls * 50);
    Ok(())
}
