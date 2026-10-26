use wasmtime::{component::Component, Config, Engine};

pub fn engine() -> wasmtime::Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    config.epoch_interruption(true);
    Engine::new(&config)
}

/// Offline preflight uses the same admission and engine as execution.
pub fn check(bytes: &[u8], world: &str) -> wasmtime::Result<Component> {
    chariox_app_wit::check_component(bytes, world)?;
    Component::new(&engine()?, bytes)
}
