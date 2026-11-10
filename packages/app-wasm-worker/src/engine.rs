use wasmtime::{component::Component, Config, Engine};

pub fn engine() -> wasmtime::Result<Engine> {
    Engine::new(&configuration())
}

pub(crate) fn configuration() -> Config {
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
    config
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
    let mut digest = Sha256::new();
    digest.update(ENGINE_CONFIG.as_bytes());
    #[cfg(target_os = "macos")]
    digest.update(b";macos-code=map-jit-v1");
    format!("sha256:{:x}", digest.finalize())
}

/// The exact executable build, read before containment removes file access.
pub fn worker_build() -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    #[cfg(target_os = "linux")]
    let mut executable = std::fs::File::open("/proc/self/exe")?;
    #[cfg(not(target_os = "linux"))]
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
    Ok(format!("sha256:{:x}", build.finalize()))
}

pub fn runtime_identity(
    engine: &Engine,
    worker_build: String,
) -> wasmtime::Result<chariox_app_runtime::release_store::aot_cache::AotRuntimeIdentity> {
    use sha2::{Digest, Sha256};
    use std::hash::{Hash, Hasher};
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
    Ok(
        chariox_app_runtime::release_store::aot_cache::AotRuntimeIdentity {
            wasmtime_version: "50.0.0".into(),
            worker_build,
            engine_config_digest: configuration_digest(),
            target_triple: env!("CHARIOX_WORKER_TARGET").into(),
            cpu_feature_policy: format!("native:sha256:{:x}", compatibility.0.finalize()),
        },
    )
}

/// A distinct identity keeps gate fuel, stack and integer features out of App
/// caches. Gates never deserialize uploaded or App-profile native code.
pub const GATE_ENGINE_CONFIG: &str = chariox_app_runtime::workflow_gate::PROFILE;
pub(crate) fn gate_configuration() -> Config {
    let mut config = configuration();
    config.wasm_component_model_async(false);
    config.consume_fuel(true);
    config.max_wasm_stack(256 * 1024);
    config
        .wasm_simd(false)
        .wasm_relaxed_simd(false)
        .wasm_gc(false)
        .wasm_memory64(false)
        .wasm_multi_memory(false)
        .wasm_tail_call(false)
        .wasm_extended_const(false);
    config
}
