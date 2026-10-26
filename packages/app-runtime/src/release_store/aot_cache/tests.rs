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
fn a_hit_does_not_contend_with_another_components_compile() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let first = component(b"release");
    cache.prepare_with(&first, &identity(), compiled).unwrap();
    let bytes = b"another signed component".to_vec();
    let second = VerifiedComponent::new(bytes.clone(), digest(&bytes), digest(b"release2"), first.world().into());
    let (started, compiling) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let cache = &cache;
        let background = scope.spawn(move || cache.prepare_with(&second, &identity(), |_| {
            started.send(()).unwrap();
            released.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            compiled(&second)
        }));
        compiling.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let hit = cache.prepare_with(&first, &identity(), |_| panic!("warm key compiled"));
        release.send(()).unwrap();
        background.join().unwrap().unwrap();
        assert!(hit.unwrap().cache_hit);
    });
}

#[test]
fn cold_compile_restart_hit_and_cross_release_reuse_keep_original_provenance() {
    let scratch = Scratch::new();
    let source = component(b"first release");
    let first = scratch
        .cache()
        .prepare_with(&source, &identity(), compiled)
        .unwrap();
    assert!(!first.cache_hit);
    let second = scratch
        .cache()
        .prepare_with(&component(b"second release"), &identity(), |_| {
            panic!("cache hit must not compile")
        })
        .unwrap();
    assert!(second.cache_hit);
    let verified = VerifiedAot::from_supervisor_descriptor(
        second.file,
        &first.key,
        &scratch.cache().public_key(),
    )
    .unwrap();
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
    let first = cache.prepare_with(&source, &identity(), compiled).unwrap();
    let path = scratch.entry(&first.key);
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(matches!(
        cache.prepare_with(&source, &identity(), |_| panic!(
            "a corrupt entry is refused"
        )),
        Err(AotError::Integrity)
    ));
    cache.evict(&first.key).unwrap();
    assert!(
        !cache
            .prepare_with(&source, &identity(), compiled)
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
        .prepare_with(&source, &identity(), compiled)
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
                &wrong,
                &scratch.cache().public_key()
            ),
            Err(AotError::Incompatible)
        ));
    }
    let mut patched = identity();
    patched.worker_build = digest(b"security update");
    let stale_under_new_name = scratch.entry(&patched.key(source.digest()));
    fs::copy(scratch.entry(&first.key), &stale_under_new_name).unwrap();
    fs::set_permissions(&stale_under_new_name, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(matches!(
        scratch.cache().prepare_with(&source, &patched, |_| panic!(
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
        .prepare_with(&source, &identity(), compiled)
        .unwrap();
    let mut patched = identity();
    patched.engine_config_digest = digest(b"updated engine configuration");
    assert!(matches!(
        scratch
            .cache()
            .prepare_with(&source, &patched, |_| Err(AotError::Compile(
                "incompatible build".into()
            ))),
        Err(AotError::Compile(_))
    ));
    let second = scratch
        .cache()
        .prepare_with(&source, &patched, |source| {
            assert_eq!(source.bytes(), b"verified portable component");
            compiled(source)
        })
        .unwrap();
    assert!(!second.cache_hit);
    assert_ne!(first.key.id(), second.key.id());
    assert!(VerifiedAot::from_supervisor_descriptor(
        second.file,
        &second.key,
        &scratch.cache().public_key()
    )
    .is_ok());
}

#[test]
fn cache_rejects_symlinks_shared_files_and_bad_permissions() {
    let scratch = Scratch::new();
    let source = component(b"release");
    let cache = scratch.cache();
    let first = cache.prepare_with(&source, &identity(), compiled).unwrap();
    let path = scratch.entry(&first.key);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cache.prepare_with(&source, &identity(), compiled).is_err());
    assert!(!path.exists());
    cache.prepare_with(&source, &identity(), compiled).unwrap();
    let outside = scratch.0.join("outside");
    fs::hard_link(&path, &outside).unwrap();
    assert!(cache.prepare_with(&source, &identity(), compiled).is_err());
    assert!(!path.exists());
    symlink(&outside, &path).unwrap();
    assert!(cache.prepare_with(&source, &identity(), compiled).is_err());
    assert!(outside.is_file());
    assert!(!path.exists());
    assert!(
        !cache
            .prepare_with(&source, &identity(), compiled)
            .unwrap()
            .cache_hit
    );
}

#[test]
fn cache_compilers_are_serialized_and_failed_compilation_publishes_nothing() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let source = component(b"release");
    assert!(matches!(
        cache.prepare_with(&source, &identity(), |_| {
            assert!(matches!(
                cache.prepare_with(&source, &identity(), compiled),
                Err(AotError::Busy)
            ));
            Err(AotError::Compile("recoverable".into()))
        }),
        Err(AotError::Compile(_))
    ));
    assert!(!scratch.entry(&identity().key(source.digest())).exists());
    assert!(
        !cache
            .prepare_with(&source, &identity(), compiled)
            .unwrap()
            .cache_hit
    );
}

#[test]
fn recomputed_digest_cannot_forge_locally_compiled_cache_provenance() {
    let scratch = Scratch::new();
    let cache = scratch.cache();
    let source = component(b"release");
    let prepared = cache.prepare_with(&source, &identity(), compiled).unwrap();
    let path = scratch.entry(&prepared.key);
    let original = fs::read(&path).unwrap();
    let length = u32::from_be_bytes(original[8..12].try_into().unwrap()) as usize;
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&original[12..12 + length]).unwrap();
    let mut artifact = original[12 + length..].to_vec();
    *artifact.last_mut().unwrap() ^= 1;
    let provenance = if metadata.get("provenance").is_some() {
        &mut metadata["provenance"]
    } else {
        &mut metadata
    };
    provenance["artifactDigest"] = digest(&artifact).into();
    let metadata = serde_json::to_vec(&metadata).unwrap();
    let mut forged = original[..8].to_vec();
    forged.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
    forged.extend_from_slice(&metadata);
    forged.extend_from_slice(&artifact);
    let mode = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, forged).unwrap();
    fs::set_permissions(&path, mode).unwrap();
    assert!(matches!(
        cache.prepare_with(&source, &identity(), |_| panic!(
            "forged native code must be refused"
        )),
        Err(AotError::Integrity)
    ));
}

#[test]
fn damaged_cache_identity_rotates_without_accepting_previous_native_entries() {
    let scratch = Scratch::new();
    let source = component(b"release");
    let cache = scratch.cache();
    let prepared = cache.prepare_with(&source, &identity(), compiled).unwrap();
    let old_key = cache.public_key();
    let key = scratch
        .entry(&prepared.key)
        .parent()
        .unwrap()
        .join(".provenance-key");
    fs::write(key, [0]).unwrap();
    drop(cache);
    let recovered = scratch.cache();
    assert_ne!(recovered.public_key(), old_key);
    assert!(matches!(
        recovered.prepare_with(&source, &identity(), |_| panic!(
            "old native entry must be refused first"
        )),
        Err(AotError::Integrity)
    ));
    assert!(
        !recovered
            .prepare_with(&source, &identity(), compiled)
            .unwrap()
            .cache_hit
    );
}
