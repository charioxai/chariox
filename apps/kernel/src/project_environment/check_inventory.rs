//! MP-08 / MP-10 / MP-11: bounded named host probes; no scripts or installers.
use super::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
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
pub(super) fn machine(root: &Path) -> EnvironmentMeasuredFacts {
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
        use std::os::unix::ffi::OsStrExt;
        if let Ok(p) = std::ffi::CString::new(root.as_os_str().as_bytes()) {
            let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if unsafe { libc::statvfs(p.as_ptr(), stat.as_mut_ptr()) } == 0 {
                let stat = unsafe { stat.assume_init() };
                f.free_disk_bytes = (stat.f_bavail as u64).checked_mul(stat.f_frsize as u64);
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
fn inventory_args(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "node" | "npm" | "pnpm" | "yarn" | "bun" | "python" | "python3" | "pip" | "pip3"
        | "ruby" | "cargo" | "rustc" | "git" | "docker" | "mise" | "deno" | "php" | "java"
        | "javac" | "gcc" | "g++" | "clang" | "cmake" | "make" | "vim" | "nvim" | "ffmpeg"
        | "pandoc" | "psql" => Some(&["--version"]),
        "go" => Some(&["version"]),
        _ => None,
    }
}
pub(super) fn executable(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name.is_empty() {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|p| p.is_absolute())
        .find_map(|p| {
            let p = p.join(name).canonicalize().ok()?;
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
}
pub(super) fn version(name: &str, roots: &[PathBuf]) -> Result<Option<String>, &'static str> {
    let Some(args) = inventory_args(name) else {
        return Err("unsupported_inventory");
    };
    let Some(path) = executable(name) else {
        return Ok(None);
    };
    if roots.iter().any(|r| path.starts_with(r)) {
        return Err("untrusted_executable");
    }
    let mut command = std::process::Command::new(&path);
    command
        .args(args)
        .current_dir("/")
        .env_clear()
        .env("LANG", "C")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin");
    let output =
        crate::local::provider_requests::capture_inventory_probe(command, Duration::from_secs(3))
            .map_err(|_| "probe_failed")?;
    if !output.status.success() {
        return Err("probe_failed");
    }
    let stdout = Zeroizing::new(output.stdout);
    let stderr = Zeroizing::new(output.stderr);
    // Project only a numeric version token; arbitrary program text is never retained.
    for bytes in [&*stdout, &*stderr] {
        let text = Zeroizing::new(String::from_utf8_lossy(bytes).into_owned());
        for word in text.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
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
pub(super) fn satisfies(version: &str, constraint: &str) -> Option<bool> {
    let v = numbers(version)?;
    let constraint = constraint.trim();
    if constraint == "*" {
        return Some(true);
    }
    if constraint.contains("||") {
        let mut accepted = false;
        for part in constraint.split("||") {
            accepted |= satisfies(version, part)?;
        }
        return Some(accepted);
    }
    let mut accepted = true;
    for token in constraint.split_whitespace() {
        let op = [">=", "<=", ">", "<", "^", "~", "="]
            .into_iter()
            .find(|p| token.starts_with(p))
            .unwrap_or("");
        let expected = token.strip_prefix(op).unwrap_or(token);
        if expected.ends_with(".*") || expected.ends_with(".x") {
            let n = numbers(&expected[..expected.len() - 2])?;
            accepted &= v.starts_with(&n);
            continue;
        }
        let e = numbers(expected)?;
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
            "~" => !cmp.is_lt() && v.get(0) == e.get(0) && (e.len() < 2 || v.get(1) == e.get(1)),
            "^" => {
                let i = e.iter().position(|n| *n != 0).unwrap_or(e.len() - 1);
                !cmp.is_lt() && v.get(..=i) == e.get(..=i)
            }
            "" | "=" => v.starts_with(&e),
            _ => return None,
        };
    }
    (!constraint.is_empty()).then_some(accepted)
}
pub(super) fn health(
    probe: &EnvironmentProbe,
    expectation: &Option<EnvironmentHealthExpectation>,
    roots: &[PathBuf],
) -> Result<bool, &'static str> {
    match probe {
        EnvironmentProbe::Tcp {
            host,
            port,
            timeout_ms,
        } => {
            use std::net::ToSocketAddrs;
            let timeout = Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64);
            // DNS is bounded through the existing resolver by HTTP; TCP literal addresses avoid opaque resolver stalls.
            let ip: std::net::IpAddr = if host == "localhost" {
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            } else {
                host.parse().map_err(|_| "tcp_requires_ip_address")?
            };
            let addr = (ip, *port)
                .to_socket_addrs()
                .map_err(|_| "probe_failed")?
                .next()
                .ok_or("probe_failed")?;
            Ok(std::net::TcpStream::connect_timeout(&addr, timeout).is_ok())
        }
        EnvironmentProbe::Http {
            url,
            expected_status,
            timeout_ms,
        } => {
            let parsed = url::Url::parse(url).map_err(|_| "invalid_probe")?;
            if !matches!(parsed.scheme(), "http" | "https")
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err("invalid_probe");
            }
            let expected = match expectation {
                Some(EnvironmentHealthExpectation::HttpStatus { status }) => *status,
                _ => *expected_status,
            };
            let agent = ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64))
                .build();
            match agent.get(url).call() {
                Ok(r) => Ok(r.status() == expected),
                Err(ureq::Error::Status(s, _)) => Ok(s == expected),
                Err(_) => Ok(false),
            }
        }
        EnvironmentProbe::Executable {
            name,
            arguments,
            timeout_ms,
        } => {
            let Some(args) = inventory_args(name) else {
                return Err("worker_required");
            };
            if arguments.iter().map(String::as_str).collect::<Vec<_>>() != args {
                return Err("worker_required");
            }
            let Some(p) = executable(name) else {
                return Ok(false);
            };
            if roots.iter().any(|r| p.starts_with(r)) {
                return Err("worker_required");
            }
            let mut c = std::process::Command::new(p);
            c.args(args)
                .current_dir("/")
                .env_clear()
                .env("LANG", "C")
                .env("PATH", "/usr/local/bin:/usr/bin:/bin");
            let exit = crate::local::provider_requests::capture_inventory_probe(
                c,
                Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64),
            )
            .map_err(|_| "probe_failed")?
            .status
            .code();
            Ok(exit
                == Some(match expectation {
                    Some(EnvironmentHealthExpectation::CommandExit { exit_code }) => *exit_code,
                    _ => 0,
                }))
        }
    }
}

/// MP-08 / MP-10 / MP-11: package and font inventory reads installed facts,
/// never lockfiles as proof, package scripts or package-manager mutations.
pub(super) fn software_version(
    name: &str,
    source: Option<&str>,
    root: Option<&Path>,
    roots: &[PathBuf],
) -> Result<Option<String>, &'static str> {
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
            let text = match super::resolver::read_environment_file(root, &path) {
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
            if roots.iter().any(|r| path.starts_with(r)) {
                return Err("untrusted_executable");
            }
            let mut c = std::process::Command::new(path);
            c.args(["--format", "%{family}\n"])
                .current_dir("/")
                .env_clear()
                .env("LANG", "C")
                .env("PATH", "/usr/local/bin:/usr/bin:/bin");
            let output =
                crate::local::provider_requests::capture_inventory_probe(c, Duration::from_secs(3))
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
            if roots.iter().any(|r| path.starts_with(r)) {
                return Err("untrusted_executable");
            }
            let mut c = std::process::Command::new(path);
            c.args(["--show", "--showformat=${Status}\t${Version}", "--", name])
                .current_dir("/")
                .env_clear()
                .env("LANG", "C")
                .env("PATH", "/usr/local/bin:/usr/bin:/bin");
            let output =
                crate::local::provider_requests::capture_inventory_probe(c, Duration::from_secs(3))
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
        _ => version(name, roots),
    }
}

pub(super) fn platform_matches(
    required: &EnvironmentPlatform,
    actual: &EnvironmentPlatform,
) -> bool {
    (required.os == "any" || required.os == actual.os)
        && (required.architecture == "any" || required.architecture == actual.architecture)
}
