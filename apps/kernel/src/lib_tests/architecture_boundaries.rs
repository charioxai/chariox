use std::path::{Path, PathBuf};

#[test]
fn runtime_command_paths_do_not_lock_daemon_app() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for path in rust_files(&src_root.join("runtime")) {
        let relative = path
            .strip_prefix(&src_root)
            .expect("runtime source should live under src")
            .to_path_buf();
        if !scan_runtime_command_path(&relative) {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("runtime source should be readable");
        let production_source = strip_cfg_test_modules(&source);
        for pattern in ["app.lock().await", "app.lock().await;"] {
            for (line_index, line) in production_source.lines().enumerate() {
                if line.contains(pattern) {
                    violations.push(format!(
                        "{}:{}: {}",
                        relative.display(),
                        line_index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    violations.sort();
    violations.dedup();
    assert!(
        violations.is_empty(),
        "runtime command paths must route app mutations through KernelRuntimeState/owned stores, not direct app.lock().await:\n{}",
        violations.join("\n")
    );
}

fn scan_runtime_command_path(relative: &Path) -> bool {
    if relative
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new("tests"))
    {
        return false;
    }
    if relative.starts_with("runtime/state") {
        return false;
    }
    if relative == Path::new("runtime/router/composition.rs") {
        return false;
    }
    relative
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("rs")
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn strip_cfg_test_modules(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(attribute_offset) = source[cursor..].find("#[cfg(test)]") {
        let attribute_start = cursor + attribute_offset;
        let after_attribute = attribute_start + "#[cfg(test)]".len();
        let Some(module_offset) = source[after_attribute..].find("mod tests") else {
            break;
        };
        let module_start = after_attribute + module_offset;
        if source[after_attribute..module_start].contains("#[") {
            cursor = after_attribute;
            continue;
        }
        let Some(open_brace_offset) = source[module_start..].find('{') else {
            break;
        };
        let open_brace = module_start + open_brace_offset;
        let Some(close_brace) = matching_brace(source, open_brace) else {
            break;
        };
        stripped.push_str(&source[cursor..attribute_start]);
        for _ in 0..source[attribute_start..=close_brace]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
        {
            stripped.push('\n');
        }
        cursor = close_brace + 1;
    }
    stripped.push_str(&source[cursor..]);
    stripped
}

fn matching_brace(source: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open_brace..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open_brace + offset);
                }
            }
            _ => {}
        }
    }
    None
}
