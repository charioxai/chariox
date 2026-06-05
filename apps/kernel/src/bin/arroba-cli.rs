use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("serve") {
        return run_serve_command(&workspace_root, &args[1..]);
    }
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
        .args(args)
        .current_dir(&workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to launch TypeScript CLI with `{bun}`: {error}"))?;

    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn run_serve_command(workspace_root: &Path, args: &[String]) -> Result<ExitCode, String> {
    if args.first().map(String::as_str) == Some("--help")
        || args.first().map(String::as_str) == Some("-h")
    {
        print_serve_help();
        return Ok(ExitCode::SUCCESS);
    }
    let package_arg = args.first().ok_or_else(|| {
        "usage: arroba serve <publication-package-or-publication.json> <port> [--host <host>] [--hook <id>] [--kernel-url <url>]".to_string()
    })?;
    let port = args.get(1).ok_or_else(|| {
        "usage: arroba serve <publication-package-or-publication.json> <port> [--host <host>] [--hook <id>] [--kernel-url <url>]".to_string()
    })?;
    let package_path = resolve_user_path(package_arg)?;
    if !package_path.exists() {
        return Err(format!(
            "published workflow package `{}` was not found",
            package_path.display()
        ));
    }

    let mut host = "127.0.0.1".to_string();
    let mut hook_id: Option<String> = None;
    let mut kernel_url: Option<String> = None;
    let mut tls_key_file: Option<String> = None;
    let mut tls_cert_file: Option<String> = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                host = require_serve_value(args, &mut index, "--host")?;
            }
            "--hook" => {
                hook_id = Some(require_serve_value(args, &mut index, "--hook")?);
            }
            "--kernel-url" => {
                kernel_url = Some(require_serve_value(args, &mut index, "--kernel-url")?);
            }
            "--tls-key-file" => {
                tls_key_file = Some(require_serve_value(args, &mut index, "--tls-key-file")?);
            }
            "--tls-cert-file" => {
                tls_cert_file = Some(require_serve_value(args, &mut index, "--tls-cert-file")?);
            }
            "--help" | "-h" => {
                print_serve_help();
                return Ok(ExitCode::SUCCESS);
            }
            option => {
                return Err(format!("unknown arroba serve option `{option}`"));
            }
        }
        index += 1;
    }

    ensure_server_built(workspace_root)?;
    let node = env::var("NODE_BIN").unwrap_or_else(|_| "node".to_string());
    let server_entry = workspace_root.join("apps/server/dist/index.js");
    let mut command = Command::new(&node);
    command
        .arg(server_entry)
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("ARROBA_PUBLICATION_PACKAGE", package_path)
        .env("HOST", host)
        .env("PORT", port);
    if let Some(value) = hook_id {
        command.env("ARROBA_PUBLICATION_HOOK_ID", value);
    }
    if let Some(value) = kernel_url {
        command.env("ARROBA_KERNEL_URL", value);
    }
    if let Some(value) = tls_key_file {
        command.env(
            "ARROBA_PUBLICATION_TLS_KEY_FILE",
            resolve_user_path(&value)?,
        );
    }
    if let Some(value) = tls_cert_file {
        command.env(
            "ARROBA_PUBLICATION_TLS_CERT_FILE",
            resolve_user_path(&value)?,
        );
    }
    run_workflow_publication_server(command, &node)
}

#[cfg(unix)]
fn run_workflow_publication_server(mut command: Command, node: &str) -> Result<ExitCode, String> {
    let error = command.exec();
    Err(format!(
        "failed to launch workflow publication server with `{node}`: {error}"
    ))
}

#[cfg(not(unix))]
fn run_workflow_publication_server(mut command: Command, node: &str) -> Result<ExitCode, String> {
    let status = command.status().map_err(|error| {
        format!("failed to launch workflow publication server with `{node}`: {error}")
    })?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn require_serve_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn resolve_user_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd =
        env::current_dir().map_err(|error| format!("failed to read current directory: {error}"))?;
    Ok(cwd.join(path))
}

fn print_serve_help() {
    println!(
        "{}",
        [
            "usage: arroba serve <publication-package-or-publication.json> <port> [options]",
            "",
            "Options:",
            "  --host <host>              Bind host (default: 127.0.0.1)",
            "  --hook <id>                Select a hook from publication.json",
            "  --kernel-url <url>         Kernel WebSocket URL",
            "  --tls-key-file <path>      Enable HTTPS with this private key",
            "  --tls-cert-file <path>     Enable HTTPS with this certificate",
            "",
        ]
        .join("\n")
    );
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

fn ensure_server_built(workspace_root: &Path) -> Result<(), String> {
    let server_dir = workspace_root.join("apps/server");
    let freshness = assess_server_build_freshness(workspace_root, &server_dir)?;

    if !freshness.needs_build {
        arroba_kernel::logging::info_with_fields(
            "cli.launcher",
            "skipping workflow publication server build because output is up to date",
            serde_json::json!({
                "dist_entry": server_dir.join("dist/index.js").display().to_string(),
            }),
        );
        return Ok(());
    }

    arroba_kernel::logging::info_with_fields(
        "cli.launcher",
        "building workflow publication server before launch",
        serde_json::json!({
            "reason": freshness.reason,
        }),
    );

    let status = Command::new("pnpm")
        .arg("--dir")
        .arg(workspace_root)
        .arg("--filter")
        .arg("@arroba/server")
        .arg("run")
        .arg("build")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            format!("failed to build the workflow publication server before launch: {error}")
        })?;

    if !status.success() {
        return Err("failed to build the workflow publication server before launch".to_string());
    }

    if server_dir.join("dist/index.js").exists() {
        Ok(())
    } else {
        Err(
            "the workflow publication server build did not produce apps/server/dist/index.js"
                .to_string(),
        )
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

fn assess_server_build_freshness(
    workspace_root: &Path,
    server_dir: &Path,
) -> Result<CliBuildFreshness, String> {
    let mut inputs = vec![
        server_dir.join("package.json"),
        server_dir.join("tsconfig.json"),
    ];
    inputs.extend(collect_typescript_sources(&server_dir.join("src"))?);

    let package_dir = workspace_root.join("packages/kernel-client");
    inputs.push(package_dir.join("package.json"));
    inputs.push(package_dir.join("tsconfig.json"));
    inputs.extend(collect_typescript_sources(&package_dir.join("src"))?);

    let outputs = vec![server_dir.join("dist/index.js")];
    let missing_output = outputs.iter().find(|path| !path.exists()).cloned();
    if let Some(path) = missing_output {
        return Ok(CliBuildFreshness {
            needs_build: true,
            reason: format!(
                "missing workflow publication server build output `{}`",
                path.display()
            ),
        });
    }

    let newest_input = newest_modified_time(&inputs)?;
    let oldest_output = oldest_modified_time(&outputs)?;

    if newest_input > oldest_output {
        return Ok(CliBuildFreshness {
            needs_build: true,
            reason: "workflow publication server sources are newer than build output".to_string(),
        });
    }

    Ok(CliBuildFreshness {
        needs_build: false,
        reason: "workflow publication server build output is current".to_string(),
    })
}

fn cli_build_inputs(cli_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut inputs = vec![
        cli_dir.join("package.json"),
        cli_dir.join("tsconfig.json"),
        cli_dir.join("scripts/build.mjs"),
    ];
    inputs.extend(collect_typescript_sources(&cli_dir.join("src"))?);

    if let Some(workspace_root) = cli_dir.parent().and_then(|apps_dir| apps_dir.parent()) {
        for package_name in ["kernel-client", "tool-display"] {
            let package_dir = workspace_root.join("packages").join(package_name);
            if !package_dir.exists() {
                continue;
            }
            inputs.push(package_dir.join("package.json"));
            inputs.push(package_dir.join("tsconfig.json"));
            inputs.extend(collect_typescript_sources(&package_dir.join("src"))?);
        }
    }
    Ok(inputs)
}

fn cli_build_outputs(cli_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let src_dir = cli_dir.join("src");
    let dist_dir = cli_dir.join("dist");
    let mut outputs = Vec::new();
    for path in collect_typescript_sources(&src_dir)? {
        let relative_path = path.strip_prefix(&src_dir).map_err(|error| {
            format!(
                "failed to resolve TypeScript CLI source path `{}`: {error}",
                path.display()
            )
        })?;
        outputs.push(dist_dir.join(relative_path).with_extension("js"));
    }
    Ok(outputs)
}

fn collect_typescript_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| {
        format!(
            "failed to read TypeScript sources in `{}`: {error}",
            root.display()
        )
    })?;
    let mut sources = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate TypeScript sources in `{}`: {error}",
                root.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            sources.extend(collect_typescript_sources(&path)?);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts") | Some("tsx")
        ) {
            sources.push(path);
        }
    }
    Ok(sources)
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
