use wasmtime::{component::Component, Config, Engine};

pub fn engine() -> wasmtime::Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    config.epoch_interruption(true);
    // Linux RLIMIT_AS bounds the whole worker, including compiler and stacks.
    // Avoid the default multi-GiB virtual reservation for each guest memory.
    config.memory_reservation(0);
    config.memory_reservation_for_growth(4 << 20);
    config.memory_guard_size(64 << 10);
    // No memfd or filesystem backing escapes the native file-size ceiling.
    config.memory_init_cow(false);
    Engine::new(&config)
}

/// Offline preflight uses the same admission and engine as execution.
pub fn check(bytes: &[u8], world: &str) -> wasmtime::Result<Component> {
    chariox_app_wit::check_component(bytes, world)?;
    Component::new(&engine()?, bytes)
}

/// Keep this identity in R4's AOT key. Platform/CPU policy and worker build
/// are independent parts of that key, never inferred from a package.
pub const ENGINE_CONFIG: &str = "wasmtime=50.0.0;component-model-async=true;epoch-interruption=true;compiler=cranelift;cpu=native;memory-reservation=0;memory-reservation-for-growth=4194304;memory-guard-size=65536;memory-init-cow=false";
pub fn configuration_digest() -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(ENGINE_CONFIG.as_bytes()))
}
