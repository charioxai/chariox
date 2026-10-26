use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "chariox-aot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn cache(&self) -> AotCache {
        AotCache::open_or_create(&self.0.join("kernel.db")).unwrap()
    }
    fn entry(&self, key: &AotCacheKey) -> PathBuf {
        fs::read_dir(&self.0)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join(&key.id()[7..])
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn component(release: &[u8]) -> VerifiedComponent {
    let bytes = b"verified portable component".to_vec();
    VerifiedComponent::new(
        bytes.clone(),
        digest(&bytes),
        digest(release),
        "chariox:app/app@1.0.0".into(),
    )
}
fn identity() -> AotRuntimeIdentity {
    AotRuntimeIdentity {
        wasmtime_version: "50.0.0".into(),
        worker_build: digest(b"worker"),
        engine_config_digest: digest(b"engine"),
        target_triple: "x86_64-unknown-linux-gnu".into(),
        cpu_feature_policy: "native:test".into(),
    }
}
fn compiled(_: &VerifiedComponent) -> Result<Vec<u8>> {
    Ok(b"locally derived native artifact".to_vec())
}

#[test]
fn cold_compile_restart_hit_and_cross_release_reuse_keep_original_provenance() {
    let scratch = Scratch::new();
    let source = component(b"first release");
    let first = scratch
        .cache()
        .prepare(&source, &identity(), compiled)
        .unwrap();
    assert!(!first.cache_hit);
    let second = scratch
        .cache()
        .prepare(&component(b"second release"), &identity(), |_| {
            panic!("cache hit must not compile")
        })
        .unwrap();
    assert!(second.cache_hit);
    let verified = VerifiedAot::from_supervisor_descriptor(second.file, &first.key).unwrap();
    assert_eq!(
        verified.provenance().source_release_digest,
        digest(b"first release")
    );
    assert_eq!(verified.bytes(), b"locally derived native artifact");
}

#[test]
fn corrupt_artifact_is_refused_before_use_and_eviction_allows_recompile() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let source = component(b"release");
    let first = cache.prepare(&source, &identity(), compiled).unwrap();
    let path = scratch.entry(&first.key);
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        cache.prepare(&source, &identity(), |_| panic!(
            "a corrupt entry is refused"
        )),
        Err(AotError::Integrity)
    ));
    cache.evict(&first.key).unwrap();
    assert!(
        !cache
            .prepare(&source, &identity(), compiled)
            .unwrap()
            .cache_hit
    );
}

#[test]
fn every_build_engine_target_cpu_and_component_key_is_fenced_before_deserializing() {
    let scratch = Scratch::new();
    let source = component(b"release");
    let first = scratch
        .cache()
        .prepare(&source, &identity(), compiled)
        .unwrap();
    for field in 0..6 {
        let mut wrong = first.key.clone();
        match field {
            0 => wrong.component_digest = digest(b"other"),
            1 => wrong.wasmtime_version.push_str("-patched"),
            2 => wrong.worker_build = digest(b"patched worker"),
            3 => wrong.engine_config_digest = digest(b"patched config"),
            4 => wrong.target_triple.push_str("-different"),
            _ => wrong.cpu_feature_policy.push_str("-different"),
        }
        assert!(matches!(
            VerifiedAot::from_supervisor_descriptor(
                File::open(scratch.entry(&first.key)).unwrap(),
                &wrong
            ),
            Err(AotError::Incompatible)
        ));
    }
    let mut patched = identity();
    patched.worker_build = digest(b"security update");
    let stale_under_new_name = scratch.entry(&patched.key(source.digest()));
    fs::copy(scratch.entry(&first.key), &stale_under_new_name).unwrap();
    fs::set_permissions(&stale_under_new_name, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        scratch.cache().prepare(&source, &patched, |_| panic!(
            "stale bytes cannot be loaded"
        )),
        Err(AotError::Incompatible)
    ));
}

#[test]
fn offline_engine_update_recompiles_retained_component_without_old_native_fallback() {
    let scratch = Scratch::new();
    let source = component(b"retained release");
    let first = scratch
        .cache()
        .prepare(&source, &identity(), compiled)
        .unwrap();
    let mut patched = identity();
    patched.engine_config_digest = digest(b"updated engine configuration");
    assert!(matches!(
        scratch
            .cache()
            .prepare(&source, &patched, |_| Err(AotError::Compile(
                "incompatible build".into()
            ))),
        Err(AotError::Compile(_))
    ));
    let second = scratch
        .cache()
        .prepare(&source, &patched, |source| {
            assert_eq!(source.bytes(), b"verified portable component");
            compiled(source)
        })
        .unwrap();
    assert!(!second.cache_hit);
    assert_ne!(first.key.id(), second.key.id());
    assert!(VerifiedAot::from_supervisor_descriptor(second.file, &second.key).is_ok());
}

#[test]
fn cache_rejects_symlinks_shared_files_and_bad_permissions() {
    let scratch = Scratch::new();
    let source = component(b"release");
    let cache = scratch.cache();
    let first = cache.prepare(&source, &identity(), compiled).unwrap();
    let path = scratch.entry(&first.key);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cache.prepare(&source, &identity(), compiled).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let outside = scratch.0.join("outside");
    fs::hard_link(&path, &outside).unwrap();
    assert!(cache.prepare(&source, &identity(), compiled).is_err());
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(cache.prepare(&source, &identity(), compiled).is_err());
}

#[test]
fn cache_compilers_are_serialized_and_failed_compilation_publishes_nothing() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let source = component(b"release");
    assert!(matches!(
        cache.prepare(&source, &identity(), |_| {
            assert!(matches!(
                cache.prepare(&source, &identity(), compiled),
                Err(AotError::Busy)
            ));
            Err(AotError::Compile("recoverable".into()))
        }),
        Err(AotError::Compile(_))
    ));
    assert!(!scratch.entry(&identity().key(source.digest())).exists());
    assert!(
        !cache
            .prepare(&source, &identity(), compiled)
            .unwrap()
            .cache_hit
    );
}
