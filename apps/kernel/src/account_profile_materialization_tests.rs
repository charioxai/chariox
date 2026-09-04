use super::*;

struct ProfileFixture {
    root: PathBuf,
    registry: ProviderAccountProfileRegistry,
    profile_id: String,
    environment: BTreeMap<String, String>,
}

impl ProfileFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-account-export-{}-{}",
            std::process::id(),
            rand::thread_rng().gen::<u64>()
        ));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(root.join("accounts.json")).unwrap();
        let profile = registry
            .create_managed("owner", "opencode", "Work")
            .unwrap();
        let environment = registry
            .resolve_environment("owner", "opencode", &profile.profile_id)
            .unwrap();
        for variable in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME"] {
            fs::create_dir_all(Path::new(&environment[variable]).join("opencode")).unwrap();
        }
        Self {
            root,
            registry,
            profile_id: profile.profile_id,
            environment,
        }
    }

    fn path(&self, variable: &str, relative: &str) -> PathBuf {
        Path::new(&self.environment[variable]).join(relative)
    }

    fn export(&self) -> Result<ProviderAccountMaterialization, DaemonError> {
        self.registry
            .export_materialization("owner", "opencode", &self.profile_id)
    }
}

impl Drop for ProfileFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove disposable account fixture");
    }
}

#[test]
fn opencode_account_export_is_independent_of_local_session_database_size() {
    let source = ProfileFixture::new();
    fs::write(
        source.path("XDG_DATA_HOME", "opencode/auth.json"),
        br#"{"fixture":"credential"}"#,
    )
    .unwrap();
    fs::write(
        source.path("XDG_CONFIG_HOME", "opencode/opencode.json"),
        br#"{"model":"openai/test"}"#,
    )
    .unwrap();
    let baseline = source.export().unwrap();
    let database = source.path("XDG_DATA_HOME", "opencode/opencode.db");
    // Sparse: exercise the real size guard without allocating or copying history.
    fs::File::create(&database)
        .unwrap()
        .set_len(3 * 1024 * 1024 * 1024)
        .unwrap();
    let with_history = source
        .export()
        .expect("local history must not prevent account export");
    assert_eq!(with_history.files, baseline.files);
    assert_eq!(
        fs::metadata(database).unwrap().len(),
        3 * 1024 * 1024 * 1024
    );
}

#[test]
fn opencode_account_round_trip_preserves_portable_config_without_runtime_data() {
    let source = ProfileFixture::new();
    fs::write(
        source.path("XDG_DATA_HOME", "opencode/auth.json"),
        b"fixture-auth",
    )
    .unwrap();
    let config_names = [
        "config",
        "config.json",
        "opencode.json",
        "opencode.jsonc",
        "tui.json",
        "tui.jsonc",
    ];
    for name in config_names {
        fs::write(
            source.path("XDG_CONFIG_HOME", &format!("opencode/{name}")),
            name,
        )
        .unwrap();
    }
    fs::File::create(source.path("XDG_STATE_HOME", "opencode/prompt-history.jsonl"))
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    let dependencies = source.path("XDG_CONFIG_HOME", "opencode/node_modules");
    fs::create_dir_all(&dependencies).unwrap();
    fs::File::create(dependencies.join("generated-package"))
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing-platform-executable", dependencies.join("bin-link"))
        .unwrap();

    let materialization = source.export().unwrap();
    let paths: Vec<_> = materialization
        .files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "data/opencode/auth.json",
            "config/opencode/config",
            "config/opencode/config.json",
            "config/opencode/opencode.json",
            "config/opencode/opencode.jsonc",
            "config/opencode/tui.json",
            "config/opencode/tui.jsonc"
        ]
    );
    let worker =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    let imported = worker
        .materialize_replica("owner", &materialization)
        .unwrap();
    let environment = worker
        .resolve_environment("owner", "opencode", &imported.profile_id)
        .unwrap();
    assert_eq!(
        fs::read(Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json")).unwrap(),
        b"fixture-auth"
    );
    for name in config_names {
        assert_eq!(
            fs::read_to_string(
                Path::new(&environment["XDG_CONFIG_HOME"])
                    .join("opencode")
                    .join(name)
            )
            .unwrap(),
            name
        );
    }
    assert!(!Path::new(&environment["XDG_STATE_HOME"])
        .join("opencode/prompt-history.jsonl")
        .exists());
    assert!(!Path::new(&environment["XDG_CONFIG_HOME"])
        .join("opencode/node_modules")
        .exists());
}

#[test]
fn opencode_portable_files_still_obey_the_transfer_size_limit() {
    for (variable, relative) in [
        ("XDG_DATA_HOME", "opencode/auth.json"),
        ("XDG_CONFIG_HOME", "opencode/opencode.jsonc"),
    ] {
        let source = ProfileFixture::new();
        fs::File::create(source.path(variable, relative))
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();
        assert!(source
            .export()
            .unwrap_err()
            .to_string()
            .contains("safety limit"));
    }
}

#[cfg(unix)]
#[test]
fn opencode_export_rejects_symlinked_portable_files_and_roots() {
    for relative in ["opencode/auth.json", "opencode"] {
        let source = ProfileFixture::new();
        let path = source.path("XDG_DATA_HOME", relative);
        let outside = source.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("auth.json"), b"not-portable").unwrap();
        let target = if relative == "opencode" {
            fs::remove_dir(&path).unwrap();
            outside
        } else {
            outside.join("auth.json")
        };
        std::os::unix::fs::symlink(target, path).unwrap();
        assert!(source.export().is_err());
    }
}
