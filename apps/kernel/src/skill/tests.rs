use super::*;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-skill-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

#[test]
fn packages_and_materializes_complete_skill_directory() {
    let source_root = temp_root("package-source");
    let materialized_root = temp_root("package-materialized");
    let skill_dir = source_root.join("browser-qa");
    fs::create_dir_all(skill_dir.join("assets").join("nested")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA workflow\n---\nUse assets/prompt.txt.\n",
    )
    .unwrap();
    fs::write(skill_dir.join("assets").join("prompt.txt"), "qa checklist").unwrap();
    fs::write(
        skill_dir.join("assets").join("nested").join("fixture.json"),
        "{\"ok\":true}",
    )
    .unwrap();

    let package = package_skill_directory(&skill_dir).unwrap();
    assert_eq!(package.metadata.name, "browser-qa");
    assert!(package.files.iter().any(|file| file.path == "SKILL.md"));
    assert!(package
        .files
        .iter()
        .any(|file| file.path == "assets/prompt.txt"));
    assert!(package
        .files
        .iter()
        .any(|file| file.path == "assets/nested/fixture.json"));

    let destination =
        package::materialize_skill_package_for_id(&materialized_root, "skill-1", &package).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("SKILL.md")).unwrap(),
        fs::read_to_string(skill_dir.join("SKILL.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(destination.join("assets").join("prompt.txt")).unwrap(),
        "qa checklist"
    );
    assert_eq!(
        package_skill_directory(&destination).unwrap().version_hash,
        package.version_hash
    );

    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(materialized_root);
}

#[test]
fn package_rejects_escaping_symlinks_and_skips_ignored_directories() {
    let root = temp_root("package-symlink");
    let skill_dir = root.join("safe");
    fs::create_dir_all(skill_dir.join(".git")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: safe\ndescription: Safe skill\n---\nBody\n",
    )
    .unwrap();
    fs::write(skill_dir.join(".git").join("config"), "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", skill_dir.join("passwd-link")).unwrap();

    #[cfg(unix)]
    {
        assert!(package_skill_directory(&skill_dir).is_err());
        fs::remove_file(skill_dir.join("passwd-link")).unwrap();
    }
    let package = package_skill_directory(&skill_dir).unwrap();
    assert!(package
        .files
        .iter()
        .all(|file| !file.path.starts_with(".git")));
    assert!(package.files.iter().all(|file| file.path != "passwd-link"));

    let _ = fs::remove_dir_all(root);
}
