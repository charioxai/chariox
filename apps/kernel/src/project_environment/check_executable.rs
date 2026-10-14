//! MP-08 / MP-10 / MP-11: named host probes retain a vetted interpreter chain.
use std::io::Read;
use std::path::{Path, PathBuf};

fn trusted(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, &'static str> {
    let parent = path
        .parent()
        .ok_or("interpreter_unavailable")?
        .canonicalize()
        .map_err(|_| "interpreter_unavailable")?;
    if roots
        .iter()
        .any(|root| path.starts_with(root) || parent.starts_with(root))
    {
        return Err("untrusted_executable");
    }
    let path = path.canonicalize().map_err(|_| "interpreter_unavailable")?;
    let metadata = path.metadata().map_err(|_| "interpreter_unavailable")?;
    if !metadata.is_file() || roots.iter().any(|root| path.starts_with(root)) {
        return Err("untrusted_executable");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("interpreter_unavailable");
        }
    }
    Ok(path)
}

/// Resolve the shebang before clearing the environment. Invoke the vetted
/// interpreter explicitly, so /usr/bin/env cannot choose a different runtime.
pub(crate) fn command(
    path: &Path,
    roots: &[PathBuf],
) -> Result<std::process::Command, &'static str> {
    let mut executable = trusted(path, roots)?;
    let mut invocation = path.to_path_buf();
    let mut arguments = Vec::<std::ffi::OsString>::new();
    let mut directories = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..8 {
        if !seen.insert(executable.clone()) {
            return Err("interpreter_cycle");
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options
            .open(&executable)
            .map_err(|_| "interpreter_unavailable")?;
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err("untrusted_executable");
        }
        let mut bytes = Vec::new();
        file.take(4096)
            .read_to_end(&mut bytes)
            .map_err(|_| "interpreter_unavailable")?;
        if !bytes.starts_with(b"#!") {
            let mut command = std::process::Command::new(executable);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.arg0(&invocation);
            }
            directories.extend([
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]);
            command
                .args(arguments)
                .current_dir("/")
                .env_clear()
                .env("LANG", "C")
                .env(
                    "PATH",
                    std::env::join_paths(directories).map_err(|_| "interpreter_unavailable")?,
                );
            return Ok(command);
        }
        let end = bytes
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("unsupported_interpreter")?;
        let line = std::str::from_utf8(&bytes[2..end])
            .map_err(|_| "unsupported_interpreter")?
            .trim();
        let (name, tail) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let mut interpreter_arguments = Vec::new();
        let interpreter = if name == "/usr/bin/env" || name == "/bin/env" {
            let mut words = tail.split_ascii_whitespace();
            let first = words.next().ok_or("unsupported_interpreter")?;
            let split = first == "-S";
            let runtime = if split {
                words.next().ok_or("unsupported_interpreter")?
            } else {
                first
            };
            if runtime.starts_with('-')
                || !runtime
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err("unsupported_interpreter");
            }
            for word in words {
                if !split || word.contains(['\'', '"', '\\']) {
                    return Err("unsupported_interpreter");
                }
                interpreter_arguments.push(std::ffi::OsString::from(word));
            }
            super::check_inventory::executable(runtime).ok_or("interpreter_unavailable")?
        } else {
            if !Path::new(name).is_absolute() {
                return Err("unsupported_interpreter");
            }
            // Native shebangs pass one optional argument, not a shell command.
            if !tail.trim().is_empty() {
                interpreter_arguments.push(tail.trim().into());
            }
            PathBuf::from(name)
        };
        let resolved = trusted(&interpreter, roots)?;
        if let Some(directory) = interpreter.parent() {
            if !directories.iter().any(|path| path == directory) {
                directories.push(directory.to_path_buf());
            }
        }
        interpreter_arguments.push(executable.into_os_string());
        interpreter_arguments.extend(arguments);
        arguments = interpreter_arguments;
        invocation = interpreter;
        executable = resolved;
    }
    Err("interpreter_chain_exceeds_bounds")
}
