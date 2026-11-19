//! MP-08 / MP-10 / MP-11: adversarial complete-package regressions.
use super::*;
// Source and execution roots intentionally differ; compare every content field.
fn assert_same_content(mut actual: CharioxSkillPackage, expected: &CharioxSkillPackage) {
    actual.metadata.path = expected.metadata.path.clone();
    assert_eq!(&actual, expected);
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skillsk-pkg-{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(root.join("source")).unwrap();
        std::fs::write(
            root.join("source/SKILL.md"),
            "---\nname: review\ndescription: review code\n---\nReview carefully.\n",
        )
        .unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn skills_v2_package_verified_resend_repairs_tampering() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let root = package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).unwrap();
    std::fs::write(root.join("SKILL.md"), "tampered").unwrap();
    // Reads still reject corrupt bytes; only the verified payload can repair it.
    assert!(package::package_skill_directory(&root).is_err());
    let repaired =
        package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).unwrap();
    assert_same_content(package::package_skill_directory(&repaired).unwrap(), &p);
}
#[test]
fn skills_v2_package_verified_resend_repairs_empty_directory() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let root = f.0.join("out/skill-1").join(&p.version_hash);
    std::fs::create_dir_all(&root).unwrap();
    let repaired =
        package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).unwrap();
    assert_same_content(package::package_skill_directory(&repaired).unwrap(), &p);
}
#[test]
fn skills_v2_package_invalid_payload_cannot_remove_a_damaged_cache() {
    let f = Fixture::new();
    let mut p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let root = f.0.join("out/skill-1").join(&p.version_hash);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("sentinel"), "preserve").unwrap();
    p.files[0].content_base64 = "not base64".into();
    assert!(package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("sentinel")).unwrap(),
        "preserve"
    );
}
#[cfg(unix)]
#[test]
fn skills_v2_package_repair_refuses_a_symlink_destination() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let parent = f.0.join("out/skill-1");
    std::fs::create_dir_all(&parent).unwrap();
    std::os::unix::fs::symlink(f.0.join("source"), parent.join(&p.version_hash)).unwrap();
    assert!(package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).is_err());
    assert_same_content(
        package::package_skill_directory(&f.0.join("source")).unwrap(),
        &p,
    );
}
#[test]
fn skills_v2_package_duplicate_paths_are_rejected() {
    let f = Fixture::new();
    let mut p = package::package_skill_directory(&f.0.join("source")).unwrap();
    p.files.push(p.files[0].clone());
    assert!(package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).is_err());
}
#[test]
fn skills_v2_package_receiver_file_count_is_bounded() {
    let f = Fixture::new();
    let mut p = package::package_skill_directory(&f.0.join("source")).unwrap();
    p.files = vec![p.files[0].clone(); 513];
    assert!(package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).is_err());
}
#[cfg(unix)]
#[test]
fn skills_v2_package_executable_modes_are_in_complete_digest() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let path = f.0.join("source/helper.sh");
    std::fs::write(&path, "#!/bin/sh\ntrue\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = package::package_skill_directory(&f.0.join("source")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let after = package::package_skill_directory(&f.0.join("source")).unwrap();
    assert_ne!(before.version_hash, after.version_hash);
    let out =
        package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &after).unwrap();
    assert_ne!(
        std::fs::metadata(out.join("helper.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}
#[cfg(unix)]
#[test]
fn skills_v2_package_nonportable_source_paths_are_rejected() {
    let f = Fixture::new();
    std::fs::write(f.0.join("source/a\\b"), "asset").unwrap();
    assert!(package::package_skill_directory(&f.0.join("source")).is_err());
}
#[cfg(unix)]
#[test]
fn skills_v2_package_escaping_symlinks_are_rejected() {
    let f = Fixture::new();
    std::fs::write(f.0.join("outside"), "not an asset").unwrap();
    std::os::unix::fs::symlink(f.0.join("outside"), f.0.join("source/link")).unwrap();
    assert!(package::package_skill_directory(&f.0.join("source")).is_err());
}

#[cfg(unix)]
#[test]
fn skills_v2_package_repair_unlinks_nested_symlinks_without_following_them() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let root = f.0.join("out/skill-1").join(&p.version_hash);
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(f.0.join("source"), root.join("outside")).unwrap();
    let repaired =
        package::materialize_skill_package_for_id(&f.0.join("out"), "skill-1", &p).unwrap();
    assert_same_content(package::package_skill_directory(&repaired).unwrap(), &p);
    assert_same_content(
        package::package_skill_directory(&f.0.join("source")).unwrap(),
        &p,
    );
}
#[test]
fn skills_v2_package_concurrent_verified_repairs_share_one_publication() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let base = f.0.join("out");
    std::fs::create_dir_all(base.join("skill-1").join(&p.version_hash)).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let base = &base;
                let p = &p;
                scope.spawn(move || {
                    barrier.wait();
                    package::materialize_skill_package_for_id(base, "skill-1", p).unwrap()
                })
            })
            .collect();
        for task in tasks {
            assert_same_content(
                package::package_skill_directory(&task.join().unwrap()).unwrap(),
                &p,
            );
        }
    });
    assert!(!std::fs::read_dir(base.join("skill-1"))
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".stage-")));
}
