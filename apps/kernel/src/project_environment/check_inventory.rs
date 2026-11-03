//! MP-08 / MP-10 / MP-11: bounded named host probes; no scripts or installers.
use super::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub(super) fn facts() -> EnvironmentMeasuredFacts {
    EnvironmentMeasuredFacts {
        platform: Some(EnvironmentPlatform {
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
        }),
        version: None,
        content_digest: None,
        present: None,
        cpus: None,
        memory_bytes: None,
        free_disk_bytes: None,
        gpu: None,
    }
}
pub(super) fn machine(root: Option<&Path>) -> EnvironmentMeasuredFacts {
    let mut f = facts();
    f.cpus = std::thread::available_parallelism()
        .ok()
        .and_then(|v| u32::try_from(v.get()).ok());
    #[cfg(unix)]
    {
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if pages > 0 && size > 0 {
            f.memory_bytes = (pages as u64).checked_mul(size as u64);
        }
        if let Some(root) = root {
            #[cfg(test)]
            if let Some(marker) = std::env::var_os("CHARIOX_MACHINE_CAPACITY_STALL_MARKER") {
                std::fs::write(marker, std::process::id().to_string()).unwrap();
                std::thread::sleep(Duration::from_secs(10));
            }
            use std::os::unix::ffi::OsStrExt;
            if let Ok(p) = std::ffi::CString::new(root.as_os_str().as_bytes()) {
                let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
                if unsafe { libc::statvfs(p.as_ptr(), stat.as_mut_ptr()) } == 0 {
                    let stat = unsafe { stat.assume_init() };
                    // statvfs field widths differ between Unix platforms.
                    #[allow(clippy::unnecessary_cast)]
                    {
                        f.free_disk_bytes =
                            (stat.f_bavail as u64).checked_mul(stat.f_frsize as u64);
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Bound memory by the current execution domain as well as hardware.
        if let Ok(membership) = std::fs::read_to_string("/proc/self/cgroup") {
            if let Some(relative) = membership.lines().find_map(|l| l.strip_prefix("0::")) {
                let relative = Path::new(relative.trim_start_matches('/'));
                if !relative
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    let base = Path::new("/sys/fs/cgroup");
                    let mut path = base.join(relative);
                    for _ in 0..64 {
                        if let Ok(value) = std::fs::read_to_string(path.join("memory.max")) {
                            if let Ok(limit) = value.trim().parse::<u64>() {
                                if limit > 0 {
                                    f.memory_bytes = f.memory_bytes.map(|n| n.min(limit));
                                }
                            }
                        }
                        if path == base || !path.pop() {
                            break;
                        }
                    }
                }
            }
        }
        // Device availability, not a promise of a usable CUDA/runtime stack.
        if Path::new("/dev/dri/renderD128").exists() {
            f.gpu = Some("render-device".into());
        }
        if Path::new("/dev/nvidia0").exists() {
            f.gpu = Some("nvidia-device".into());
        }
    }
    f
}
/// Named inventory probes have fixed argv. Imported executable/arguments never widen this list.
pub(super) fn inventory_args(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "node" | "npm" | "pnpm" | "yarn" | "bun" | "python" | "python3" | "pip" | "pip3"
        | "ruby" | "cargo" | "rustc" | "git" | "docker" | "mise" | "deno" | "php" | "java"
        | "javac" | "gcc" | "g++" | "clang" | "cmake" | "make" | "vim" | "nvim" | "ffmpeg"
        | "pandoc" | "psql" => Some(&["--version"]),
        "go" => Some(&["version"]),
        _ => None,
    }
}
pub(crate) fn executable(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name.is_empty() {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|p| p.is_absolute())
        .find_map(|p| {
            let p = p.join(name);
            let m = p.metadata().ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            m.is_file().then_some(p)
        })
        // MP-08 / MP-10 / MP-11: Linux distributions commonly expose only
        // python3. Preserve an explicit python choice, then probe that alias.
        .or_else(|| (name == "python").then(|| executable("python3")).flatten())
}
pub(super) fn remaining(deadline: Instant, cap: Duration) -> Result<Duration, &'static str> {
    let left = deadline.saturating_duration_since(Instant::now()).min(cap);
    if left.is_zero() {
        Err("check_budget_exceeded")
    } else {
        Ok(left)
    }
}
#[cfg(test)]
pub(super) fn version(name: &str, roots: &[PathBuf]) -> Result<Option<String>, &'static str> {
    version_until(name, roots, Instant::now() + Duration::from_secs(3))
}
pub(super) fn version_until(
    name: &str,
    roots: &[PathBuf],
    deadline: Instant,
) -> Result<Option<String>, &'static str> {
    // The manifest runtime identity stays portable; the host compiler has a fixed probe.
    let name = if name == "rust" { "rustc" } else { name };
    let Some(args) = inventory_args(name) else {
        return Err("unsupported_inventory");
    };
    let Some(path) = executable(name) else {
        return Ok(None);
    };
    let mut command = super::check_executable::command(&path, roots)?;
    command.args(args);
    if matches!(name, "cargo" | "rustc") {
        for key in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    let output = crate::local::provider_requests::capture_inventory_probe(
        command,
        remaining(deadline, Duration::from_secs(3))?,
    )
    .map_err(|_| "probe_failed")?;
    if !output.status.success() {
        return Err("probe_failed");
    }
    let stdout = Zeroizing::new(output.stdout);
    let stderr = Zeroizing::new(output.stderr);
    // Project only stable numeric versions; prerelease suffixes cannot prove a stable bound.
    for bytes in [&*stdout, &*stderr] {
        let text = Zeroizing::new(String::from_utf8_lossy(bytes).into_owned());
        // Parenthesized text is vendor or package metadata, such as `(Ubuntu 13.3.0-6ubuntu2~24.04)`.
        let mut depth = 0usize;
        for word in text.split_whitespace() {
            let inside = depth > 0 || word.starts_with('(');
            depth = (depth + word.matches('(').count()).saturating_sub(word.matches(')').count());
            if inside {
                continue;
            }
            let word = word
                .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '.')
                .trim_end_matches(['"', '\'', '(', ')', '[', ']', ',', ';']);
            if word.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && word.contains('.')
                && word.contains(['-', '+'])
            {
                return Err("version_unavailable");
            }
            if word.contains('.')
                && word.len() <= 40
                && word.bytes().all(|c| c.is_ascii_digit() || c == b'.')
                && word
                    .split('.')
                    .all(|v| !v.is_empty() && v.parse::<u32>().is_ok())
            {
                return Ok(Some(word.into()));
            }
        }
    }
    Err("version_unavailable")
}
fn numbers(v: &str) -> Option<Vec<u32>> {
    v.trim()
        .trim_start_matches('v')
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .filter(|v| !v.is_empty() && v.len() <= 4)
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VersionSemantics {
    Numeric,
    Npm,
}
impl VersionSemantics {
    pub(super) fn software(identity: &str, source: Option<&str>) -> Self {
        if source == Some("npm package")
            || matches!(identity, "node" | "npm" | "pnpm" | "yarn" | "bun" | "deno")
        {
            Self::Npm
        } else {
            Self::Numeric
        }
    }
}
pub(super) fn satisfies(
    version: &str,
    constraint: &str,
    semantics: VersionSemantics,
) -> Option<bool> {
    let v = numbers(version)?;
    let constraint = constraint.trim();
    if constraint == "*" {
        return Some(true);
    }
    if constraint.contains("||") {
        let mut accepted = false;
        for part in constraint.split("||") {
            accepted |= satisfies(version, part, semantics)?;
        }
        return Some(accepted);
    }
    // Python conjunctions use commas and exact equality uses two equals.
    // Empty clauses and unsupported operators remain unproven.
    if constraint.split(',').any(|clause| clause.trim().is_empty()) {
        return None;
    }
    let mut accepted = true;
    for token in constraint
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|token| !token.is_empty())
    {
        let op = [">=", "<=", "==", ">", "<", "^", "~", "="]
            .into_iter()
            .find(|p| token.starts_with(p))
            .unwrap_or("");
        let expected = token.strip_prefix(op).unwrap_or(token);
        if expected.ends_with(".*") || expected.ends_with(".x") {
            let n = numbers(&expected[..expected.len() - 2])?;
            if n.len() > 2 {
                return None;
            }
            let mut lower = n.clone();
            lower.resize(4, 0);
            let mut upper = lower.clone();
            let index = if op == "^" {
                n.iter().position(|p| *p != 0).unwrap_or(n.len() - 1)
            } else {
                n.len() - 1
            };
            upper[index] = upper[index].checked_add(1)?;
            upper[index + 1..].fill(0);
            let mut actual = v.clone();
            actual.resize(4, 0);
            accepted &= match op {
                "<" => actual < lower,
                ">=" => actual >= lower,
                ">" => actual >= upper,
                "<=" => actual < upper,
                "" | "=" | "==" | "~" | "^" => actual >= lower && actual < upper,
                _ => return None,
            };
            continue;
        }
        let e = numbers(expected)?;
        // MP-08 / MP-10: partial strict/upper comparators have the same bounds
        // as their explicit wildcard form; zero padding changes npm's meaning.
        if semantics == VersionSemantics::Npm && e.len() < 3 && matches!(op, ">" | "<=") {
            accepted &= satisfies(version, &format!("{op}{expected}.*"), semantics)?;
            continue;
        }
        let padded = |n: &[u32]| {
            let mut n = n.to_vec();
            n.resize(4, 0);
            n
        };
        let cmp = padded(&v).cmp(&padded(&e));
        accepted &= match op {
            ">=" => !cmp.is_lt(),
            "<=" => !cmp.is_gt(),
            ">" => cmp.is_gt(),
            "<" => cmp.is_lt(),
            "~" => !cmp.is_lt() && v.first() == e.first() && (e.len() < 2 || v.get(1) == e.get(1)),
            "^" => {
                let i = e.iter().position(|n| *n != 0).unwrap_or(e.len() - 1);
                !cmp.is_lt() && v.get(..=i) == e.get(..=i)
            }
            "==" => cmp.is_eq(),
            "" | "=" => v.starts_with(&e),
            _ => return None,
        };
    }
    (!constraint.is_empty()).then_some(accepted)
}

/// The admitted numeric grammar consists of unions of intervals/prefixes.
/// A nonempty intersection contains zero, a declared boundary, or its successor.
/// Unknown syntax stays unproven for Check rather than becoming a Save conflict.
pub(super) fn constraints_overlap(
    a: Option<&str>,
    b: Option<&str>,
    semantics: VersionSemantics,
) -> Option<bool> {
    let (Some(a), Some(b)) = (a, b) else {
        return Some(true);
    };
    satisfies("0.0.0.0", a, semantics)?;
    satisfies("0.0.0.0", b, semantics)?;
    let mut candidates = std::collections::BTreeSet::from([[0u32; 4]]);
    for token in [a, b]
        .into_iter()
        .flat_map(|c| c.split("||"))
        .flat_map(|clause| clause.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|token| !token.is_empty())
    {
        if token == "*" {
            continue;
        }
        let token = token.trim_start_matches(['>', '<', '=', '^', '~']);
        let token = token
            .strip_suffix(".*")
            .or_else(|| token.strip_suffix(".x"))
            .unwrap_or(token);
        let n = numbers(token)?;
        if n.len() > 4 || candidates.len() > 512 {
            return None;
        }
        let mut bound = [0; 4];
        bound[..n.len()].copy_from_slice(&n);
        candidates.insert(bound);
        for i in (0..4).rev() {
            if let Some(next) = bound[i].checked_add(1) {
                // Wildcard, caret and tilde operators may start at an upper
                // prefix boundary (e.g. >22.x starts at 23), not the numeric successor.
                let mut successor = bound;
                successor[i] = next;
                successor[i + 1..].fill(0);
                candidates.insert(successor);
            }
        }
    }
    for candidate in candidates {
        let version = candidate.map(|n| n.to_string()).join(".");
        if satisfies(&version, a, semantics)? && satisfies(&version, b, semantics)? {
            return Some(true);
        }
    }
    Some(false)
}
#[cfg(test)]
pub(super) use super::check_health::health;
pub(super) use super::check_health::health_until;

/// MP-08 / MP-10 / MP-11: package and font inventory reads installed facts,
/// never lockfiles as proof, package scripts or package-manager mutations.
#[cfg(test)]
pub(super) fn software_version(
    name: &str,
    source: Option<&str>,
    root: Option<&Path>,
    roots: &[PathBuf],
) -> Result<Option<String>, &'static str> {
    software_version_until(
        name,
        source,
        root,
        roots,
        Instant::now() + Duration::from_secs(3),
    )
}
pub(super) fn software_version_until(
    name: &str,
    source: Option<&str>,
    root: Option<&Path>,
    roots: &[PathBuf],
    deadline: Instant,
) -> Result<Option<String>, &'static str> {
    remaining(deadline, Duration::from_secs(3))?;
    match source {
        Some("npm package") => {
            if name.starts_with('-')
                || name
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b))
            {
                return Err("invalid_package");
            }
            let root = root.ok_or("folder_unavailable")?;
            let path = format!("node_modules/{name}/package.json");
            let text = match super::check_package_inventory::installed_manifest(root, name) {
                Ok(t) => t,
                Err(_) => {
                    return if root.join(&path).exists() {
                        Err("package_unproven")
                    } else {
                        Ok(None)
                    }
                }
            };
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| "package_unproven")?;
            if value.get("name").and_then(|v| v.as_str()) != Some(name) {
                return Err("package_identity_mismatch");
            }
            let version = value
                .get("version")
                .and_then(|v| v.as_str())
                .filter(|v| numbers(v).is_some())
                .ok_or("package_version_unproven")?;
            Ok(Some(version.into()))
        }
        Some("font") => {
            let Some(path) = executable("fc-list") else {
                return Err("font_inventory_unavailable");
            };
            let mut c = super::check_executable::command(&path, roots)?;
            c.args(["--format", "%{family}\n"]);
            let output = crate::local::provider_requests::capture_inventory_probe(
                c,
                remaining(deadline, Duration::from_secs(3))?,
            )
            .map_err(|_| "probe_failed")?;
            if !output.status.success() {
                return Err("probe_failed");
            }
            let bytes = Zeroizing::new(output.stdout);
            let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
            Ok(text
                .lines()
                .flat_map(|l| l.split(','))
                .any(|f| f.trim().eq_ignore_ascii_case(name))
                .then(|| "present".into()))
        }
        Some("apt") | Some("apt package") => {
            if name.starts_with('-')
                || name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+-:".contains(&b))
            {
                return Err("invalid_package");
            }
            let Some(path) = executable("dpkg-query") else {
                return Err("package_inventory_unavailable");
            };
            let mut c = super::check_executable::command(&path, roots)?;
            c.args(["--show", "--showformat=${Status}\t${Version}", "--", name]);
            let output = crate::local::provider_requests::capture_inventory_probe(
                c,
                remaining(deadline, Duration::from_secs(3))?,
            )
            .map_err(|_| "probe_failed")?;
            if !output.status.success() {
                return Ok(None);
            }
            let bytes = Zeroizing::new(output.stdout);
            let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
            let Some(v) = text.strip_prefix("install ok installed\t") else {
                return Ok(None);
            };
            let v = v
                .trim()
                .split(':')
                .next_back()
                .unwrap_or(v)
                .split('-')
                .next()
                .unwrap_or(v);
            if numbers(v).is_none() {
                return Err("package_version_unproven");
            }
            Ok(Some(v.into()))
        }
        Some("manifest package") => Err("package_inventory_unavailable"),
        _ => version_until(name, roots, deadline),
    }
}

pub(super) fn platform_matches(
    required: &EnvironmentPlatform,
    actual: &EnvironmentPlatform,
) -> bool {
    (required.os == "any" || required.os == actual.os)
        && (required.architecture == "any" || required.architecture == actual.architecture)
}
