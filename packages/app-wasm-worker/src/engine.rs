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

pub fn runtime_identity(
    engine: &Engine,
) -> wasmtime::Result<chariox_app_runtime::release_store::aot_cache::AotRuntimeIdentity> {
    use sha2::{Digest, Sha256};
    use std::{
        hash::{Hash, Hasher},
        io::Read,
    };
    struct Compatibility(Sha256);
    impl Hasher for Compatibility {
        fn write(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }
        fn finish(&self) -> u64 {
            u64::from_le_bytes(self.0.clone().finalize()[..8].try_into().unwrap())
        }
    }
    let mut compatibility = Compatibility(Sha256::new());
    engine
        .precompile_compatibility_hash()
        .hash(&mut compatibility);
    let mut executable = std::fs::File::open(std::env::current_exe()?)?;
    let mut build = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = executable.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        build.update(&buffer[..size]);
    }
    Ok(
        chariox_app_runtime::release_store::aot_cache::AotRuntimeIdentity {
            wasmtime_version: "50.0.0".into(),
            worker_build: format!("sha256:{:x}", build.finalize()),
            engine_config_digest: configuration_digest(),
            target_triple: env!("CHARIOX_WORKER_TARGET").into(),
            cpu_feature_policy: format!("native:sha256:{:x}", compatibility.0.finalize()),
        },
    )
}
