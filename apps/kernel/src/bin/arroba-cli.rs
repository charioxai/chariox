use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::SystemTime;

fn main() -> ExitCode {
    let _ = arroba_kernel::logging::init_process_logger("cli-launcher");
    match run() {
        Ok(code) => {
            arroba_kernel::logging::info("cli.launcher", "TypeScript CLI exited");
            code
        }
        Err(message) => {
            arroba_kernel::logging::error_with_fields(
                "cli.launcher",
                "TypeScript CLI launcher failed",
                serde_json::json!({ "error": message }),
            );
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| format!("failed to locate workspace root: {error}"))?;
    let cli_dir = workspace_root.join("apps/cli");

    let bun = env::var("BUN_BIN").unwrap_or_else(|_| "bun".to_string());
    arroba_kernel::logging::info_with_fields(
        "cli.launcher",
        "launching TypeScript CLI",
        serde_json::json!({
            "workspace_root": workspace_root.display().to_string(),
            "bun_bin": bun.clone(),
        }),
    );

    ensure_bun_available(&bun)?;
    ensure_cli_built(&workspace_root, &bun)?;

    let status = Command::new(&bun)
        .arg(cli_dir.join("dist/index.js"))
        .args(env::args().skip(1))
        .current_dir(&workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to launch TypeScript CLI with `{bun}`: {error}"))?;

    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn ensure_bun_available(bun: &str) -> Result<(), String> {
    let status = Command::new(bun)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| {
            "Arroba's TypeScript CLI requires Bun. Install Bun or set BUN_BIN to its executable path."
                .to_string()
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(
            "Arroba's TypeScript CLI requires Bun. Install Bun or set BUN_BIN to its executable path."
                .to_string(),
        )
    }
}

fn ensure_cli_built(workspace_root: &PathBuf, bun: &str) -> Result<(), String> {
    let cli_dir = workspace_root.join("apps/cli");
    let freshness = assess_cli_build_freshness(&cli_dir)?;

    if !freshness.needs_build {
        arroba_kernel::logging::info_with_fields(
            "cli.launcher",
            "skipping TypeScript CLI build because output is up to date",
            serde_json::json!({
                "dist_entry": cli_dir.join("dist/index.js").display().to_string(),
            }),
        );
        return Ok(());
    }

    arroba_kernel::logging::info_with_fields(
        "cli.launcher",
        "building TypeScript CLI before launch",
        serde_json::json!({
            "reason": freshness.reason,
        }),
    );

    let status = Command::new("pnpm")
        .arg("--dir")
        .arg(workspace_root)
        .arg("--filter")
        .arg("@arroba/cli")
        .arg("run")
        .arg("build")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to build the TypeScript CLI before launch: {error}"))?;

    if !status.success() {
        return Err("failed to build the TypeScript CLI before launch".to_string());
    }

    if cli_dir.join("dist/index.js").exists() {
        Ok(())
    } else {
        Err(format!(
            "the TypeScript CLI build did not produce apps/cli/dist/index.js; rerun `pnpm --filter @arroba/cli run build` and then `{} apps/cli/dist/index.js` manually if needed",
            bun
        ))
    }
}

struct CliBuildFreshness {
    needs_build: bool,
    reason: String,
}

fn assess_cli_build_freshness(cli_dir: &Path) -> Result<CliBuildFreshness, String> {
    let inputs = cli_build_inputs(cli_dir)?;
    let outputs = cli_build_outputs(cli_dir)?;

    let missing_output = outputs.iter().find(|path| !path.exists()).cloned();
    if let Some(path) = missing_output {
        return Ok(CliBuildFreshness {
            needs_build: true,
            reason: format!("missing CLI build output `{}`", path.display()),
        });
    }

    let newest_input = newest_modified_time(&inputs)?;
    let oldest_output = oldest_modified_time(&outputs)?;

    if newest_input > oldest_output {
        return Ok(CliBuildFreshness {
            needs_build: true,
            reason: "CLI sources are newer than build output".to_string(),
        });
    }

    Ok(CliBuildFreshness {
        needs_build: false,
        reason: "CLI build output is current".to_string(),
    })
}

fn cli_build_inputs(cli_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut inputs = vec![
        cli_dir.join("package.json"),
        cli_dir.join("tsconfig.json"),
        cli_dir.join("scripts/build.mjs"),
    ];
    let src_dir = cli_dir.join("src");
    let entries = fs::read_dir(&src_dir).map_err(|error| {
        format!(
            "failed to read TypeScript CLI sources in `{}`: {error}",
            src_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate TypeScript CLI sources in `{}`: {error}",
                src_dir.display()
            )
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts") | Some("tsx")
        ) {
            inputs.push(path);
        }
    }
    Ok(inputs)
}

fn cli_build_outputs(cli_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let src_dir = cli_dir.join("src");
    let dist_dir = cli_dir.join("dist");
    let entries = fs::read_dir(&src_dir).map_err(|error| {
        format!(
            "failed to read TypeScript CLI sources in `{}`: {error}",
            src_dir.display()
        )
    })?;
    let mut outputs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate TypeScript CLI sources in `{}`: {error}",
                src_dir.display()
            )
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !(file_name.ends_with(".ts") || file_name.ends_with(".tsx")) {
            continue;
        }
        outputs.push(dist_dir.join(file_name.replace(".tsx", ".js").replace(".ts", ".js")));
    }
    Ok(outputs)
}

fn newest_modified_time(paths: &[PathBuf]) -> Result<SystemTime, String> {
    let mut newest = SystemTime::UNIX_EPOCH;
    for path in paths {
        let modified = fs::metadata(path)
            .map_err(|error| format!("failed to read metadata for `{}`: {error}", path.display()))?
            .modified()
            .map_err(|error| {
                format!(
                    "failed to read modified time for `{}`: {error}",
                    path.display()
                )
            })?;
        newest = newest.max(modified);
    }
    Ok(newest)
}

fn oldest_modified_time(paths: &[PathBuf]) -> Result<SystemTime, String> {
    let mut oldest = SystemTime::now();
    for path in paths {
        let modified = fs::metadata(path)
            .map_err(|error| format!("failed to read metadata for `{}`: {error}", path.display()))?
            .modified()
            .map_err(|error| {
                format!(
                    "failed to read modified time for `{}`: {error}",
                    path.display()
                )
            })?;
        oldest = oldest.min(modified);
    }
    Ok(oldest)
}

#[cfg(test)]
mod tests {
    use super::assess_cli_build_freshness;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn freshness_requires_build_when_dist_output_is_missing() {
        let cli_dir = create_test_cli_dir("missing-dist");

        let freshness =
            assess_cli_build_freshness(&cli_dir).expect("freshness check should succeed");

        assert!(freshness.needs_build);
        assert!(freshness.reason.contains("missing CLI build output"));
    }

    #[test]
    fn freshness_skips_build_when_outputs_are_newer_than_inputs() {
        let cli_dir = create_test_cli_dir("fresh");
        write_cli_outputs(&cli_dir);

        let freshness =
            assess_cli_build_freshness(&cli_dir).expect("freshness check should succeed");

        assert!(!freshness.needs_build);
    }

    #[test]
    fn freshness_requires_build_when_source_is_newer_than_output() {
        let cli_dir = create_test_cli_dir("stale");
        write_cli_outputs(&cli_dir);
        thread::sleep(Duration::from_millis(20));
        fs::write(cli_dir.join("src/index.ts"), "export const value = 2\n")
            .expect("source should update");

        let freshness =
            assess_cli_build_freshness(&cli_dir).expect("freshness check should succeed");

        assert!(freshness.needs_build);
        assert_eq!(freshness.reason, "CLI sources are newer than build output");
    }

    fn create_test_cli_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arroba-cli-launcher-{label}-{unique}"));
        fs::create_dir_all(root.join("src")).expect("src dir should exist");
        fs::create_dir_all(root.join("scripts")).expect("scripts dir should exist");
        fs::write(root.join("package.json"), "{}\n").expect("package.json should exist");
        fs::write(root.join("tsconfig.json"), "{}\n").expect("tsconfig should exist");
        fs::write(root.join("scripts/build.mjs"), "export {}\n")
            .expect("build script should exist");
        fs::write(root.join("src/index.ts"), "export const value = 1\n")
            .expect("index should exist");
        fs::write(
            root.join("src/view.tsx"),
            "export const View = () => null\n",
        )
        .expect("tsx source should exist");
        root
    }

    fn write_cli_outputs(cli_dir: &Path) {
        fs::create_dir_all(cli_dir.join("dist")).expect("dist dir should exist");
        thread::sleep(Duration::from_millis(20));
        fs::write(cli_dir.join("dist/index.js"), "export const value = 1;\n")
            .expect("index output should exist");
        fs::write(
            cli_dir.join("dist/view.js"),
            "export const View = () => null;\n",
        )
        .expect("view output should exist");
    }
}
