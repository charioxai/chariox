use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
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
            format!(
                "Arroba's TypeScript CLI requires Bun. Install Bun or set BUN_BIN to its executable path."
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Arroba's TypeScript CLI requires Bun. Install Bun or set BUN_BIN to its executable path."
        ))
    }
}

fn ensure_cli_built(workspace_root: &PathBuf, bun: &str) -> Result<(), String> {
    let dist_entry = workspace_root.join("apps/cli/dist/index.js");

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

    if dist_entry.exists() {
        Ok(())
    } else {
        Err(format!(
            "the TypeScript CLI build did not produce apps/cli/dist/index.js; rerun `pnpm --filter @arroba/cli run build` and then `{} apps/cli/dist/index.js` manually if needed",
            bun
        ))
    }
}
