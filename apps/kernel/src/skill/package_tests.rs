//! MP-08 / MP-10 / MP-11: adversarial complete-package regressions.
use super::*;
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
fn skills_v2_package_cache_tampering_is_rejected() {
    let f = Fixture::new();
    let p = package::package_skill_directory(&f.0.join("source")).unwrap();
    let root = package::materialize_skill_package(&f.0.join("out"), &p).unwrap();
    std::fs::write(root.join("SKILL.md"), "tampered").unwrap();
    assert!(package::materialize_skill_package(&f.0.join("out"), &p).is_err());
}
#[test]
fn skills_v2_package_duplicate_paths_are_rejected() {
    let f = Fixture::new();
    let mut p = package::package_skill_directory(&f.0.join("source")).unwrap();
    p.files.push(p.files[0].clone());
    assert!(package::materialize_skill_package(&f.0.join("out"), &p).is_err());
}
#[test]
fn skills_v2_package_receiver_file_count_is_bounded() {
    let f = Fixture::new();
    let mut p = package::package_skill_directory(&f.0.join("source")).unwrap();
    p.files = vec![p.files[0].clone(); 513];
    assert!(package::materialize_skill_package(&f.0.join("out"), &p).is_err());
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
    let out = package::materialize_skill_package(&f.0.join("out"), &after).unwrap();
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
