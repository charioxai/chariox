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

/// Keep this identity in R4's AOT key. Platform/CPU policy and worker build
/// are independent parts of that key, never inferred from a package.
pub const ENGINE_CONFIG: &str = "wasmtime=50.0.0;component-model-async=true;epoch-interruption=true;compiler=cranelift;cpu=native";
pub fn configuration_digest() -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(ENGINE_CONFIG.as_bytes()))
}
