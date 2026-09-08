//! Early developer dispatch: no workspace lookup, Bun build, kernel or cwd change.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

pub fn dispatch(args: &[String], protocol: u32) -> Option<Result<ExitCode, String>> {
    if args.first().map(String::as_str) != Some("app")
        || !args.get(1).is_some_and(|action| {
            matches!(
                action.as_str(),
                "create" | "keygen" | "manifest" | "pack" | "validate" | "inspect"
            )
        })
    {
        return None;
    }
    Some(run(&args[1..], protocol))
}

fn command_args(args: &[String], protocol: u32) -> Result<Vec<String>, String> {
    if args.len() > 64
        || args
            .iter()
            .any(|arg| arg.len() > 4096 || arg.contains('\0'))
    {
        return Err("App developer command arguments exceed their bounds".into());
    }
    if args
        .iter()
        .any(|arg| arg == "--kernel-protocol" || arg.starts_with("--kernel-protocol="))
    {
        return Err("The Chariox CLI supplies its current protocol; use chariox-app-package directly to select another contract.".into());
    }
    let mut forwarded = args.to_vec();
    if matches!(
        args.first().map(String::as_str),
        Some("create" | "manifest" | "pack" | "validate")
    ) && !args.iter().any(|arg| arg == "--help")
    {
        forwarded.extend(["--kernel-protocol".into(), protocol.to_string()]);
    }
    Ok(forwarded)
}

fn select_binary(explicit: Option<&Path>, launcher: &Path) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if !path.is_absolute() || !path.is_file() {
            return Err("CHARIOX_APP_PACKAGE_BIN must be an absolute executable file path".into());
        }
        return Ok(path.to_owned());
    }
    let sibling = launcher
        .parent()
        .ok_or("cannot locate App developer tool")?
        .join("chariox-app-package");
    if sibling.is_file() {
        return Ok(sibling);
    }
    let installed = PathBuf::from("/usr/local/bin/chariox-app-package");
    if installed.is_file() {
        return Ok(installed);
    }
    Err("The App developer tool is not installed. Install the matching Chariox release, or build chariox-app-package and set CHARIOX_APP_PACKAGE_BIN to its absolute path.".into())
}

fn run(args: &[String], protocol: u32) -> Result<ExitCode, String> {
    let args = command_args(args, protocol)?;
    let explicit = env::var_os("CHARIOX_APP_PACKAGE_BIN")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let executable = select_binary(
        explicit.as_deref(),
        &env::current_exe().map_err(|_| "cannot locate CLI executable")?,
    )?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // Keep original cwd so all developer-local relative paths mean what the
    // user typed. Replacing the launcher preserves status and signal behavior.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(format!(
            "App developer tool could not run: {}",
            command.exec()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("App developer tool could not run: {error}"))?;
        Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_keep_local_paths_literal_and_add_only_current_protocol() {
        let args = vec![
            "create".into(),
            "project ; $(touch forbidden)".into(),
            "--publisher".into(),
            "../public.json".into(),
        ];
        let mut expected = args.clone();
        expected.extend(["--kernel-protocol".into(), "500".into()]);
        assert_eq!(command_args(&args, 500).unwrap(), expected);
        assert!(command_args(
            &["pack".into(), "--kernel-protocol".into(), "1".into()],
            500
        )
        .is_err());
        assert_eq!(
            command_args(&["create".into(), "--help".into()], 500).unwrap(),
            ["create", "--help"]
        );
        assert!(dispatch(&["app".into(), "list".into()], 500).is_none());
    }
    #[test]
    fn explicit_relative_or_missing_helper_never_falls_back() {
        assert!(select_binary(
            Some(Path::new("chariox-app-package")),
            Path::new("/trusted/bin/chariox-cli")
        )
        .is_err());
        assert!(select_binary(
            Some(Path::new("/definitely-missing-chariox-app-package")),
            Path::new("/trusted/bin/chariox-cli")
        )
        .is_err());
    }
}
