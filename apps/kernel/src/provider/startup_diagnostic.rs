use std::collections::BTreeSet;
use std::io::Read;
use std::process::Child;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Diagnostic {
    bytes: u64,
    classes: BTreeSet<&'static str>,
    finished: bool,
}

// Provider startup output can contain account values, config, URLs or paths.
// Only closed diagnostic classes and a byte count leave this private reader.
pub(super) struct ProviderStartupDiagnostic(Arc<Mutex<Diagnostic>>);

impl ProviderStartupDiagnostic {
    pub(super) fn capture(child: &mut Child) -> Self {
        let diagnostic = Arc::new(Mutex::new(Diagnostic::default()));
        if let Some(mut stderr) = child.stderr.take() {
            let captured = diagnostic.clone();
            let reader = std::thread::Builder::new()
                .name("provider-startup-stderr".to_string())
                .spawn(move || {
                    let mut chunk = [0; 4096];
                    let mut tail = Vec::new();
                    while let Ok(count) = stderr.read(&mut chunk) {
                        if count == 0 {
                            break;
                        }
                        tail.extend_from_slice(&chunk[..count]);
                        if let Ok(mut diagnostic) = captured.lock() {
                            diagnostic.bytes = diagnostic.bytes.saturating_add(count as u64);
                            classify(&tail, &mut diagnostic.classes);
                        }
                        // Retain overlap for split phrases, never whole output.
                        if tail.len() > 256 {
                            tail.drain(..tail.len() - 256);
                        }
                    }
                    if let Ok(mut diagnostic) = captured.lock() {
                        diagnostic.finished = true;
                    }
                });
            if reader.is_err() {
                if let Ok(mut diagnostic) = diagnostic.lock() {
                    diagnostic.classes.insert("diagnostic_reader_unavailable");
                    diagnostic.finished = true;
                }
            }
        }
        Self(diagnostic)
    }

    pub(super) fn summary(&self) -> String {
        // Called only after early exit or owned timeout cleanup. Give the
        // reader a bounded chance to consume final bytes without joining a
        // potentially inherited pipe writer.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        while std::time::Instant::now() < deadline {
            if self
                .0
                .lock()
                .map(|diagnostic| diagnostic.finished)
                .unwrap_or(true)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let Ok(diagnostic) = self.0.lock() else {
            return "startup diagnostic unavailable".to_string();
        };
        format_summary(&diagnostic)
    }
}

pub(crate) fn summarize_provider_startup_output(output: &[u8]) -> String {
    let mut diagnostic = Diagnostic {
        bytes: output.len() as u64,
        ..Diagnostic::default()
    };
    classify(output, &mut diagnostic.classes);
    format_summary(&diagnostic)
}

fn format_summary(diagnostic: &Diagnostic) -> String {
    format!(
        "stderr_bytes={} diagnostic_classes={}",
        diagnostic.bytes,
        if diagnostic.classes.is_empty() {
            "unclassified".to_string()
        } else {
            diagnostic
                .classes
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(",")
        },
    )
}

fn classify(output: &[u8], classes: &mut BTreeSet<&'static str>) {
    let output = String::from_utf8_lossy(output).to_ascii_lowercase();
    for (class, phrases) in [
        (
            "permission_denied",
            &["permission denied", "operation not permitted"][..],
        ),
        (
            "configuration",
            &[
                "error loading config",
                "failed to parse",
                "invalid config",
                "unknown variant",
            ][..],
        ),
        (
            "unsupported_arguments",
            &[
                "unexpected argument",
                "unrecognized option",
                "unknown option",
            ][..],
        ),
        ("address_in_use", &["address already in use"][..]),
        ("missing_file", &["no such file or directory"][..]),
        (
            "namespace",
            &[
                "creating new namespace",
                "failed to create namespace",
                "bwrap:",
            ][..],
        ),
        (
            "authentication",
            &[
                "authentication failed",
                "unauthorized",
                "failed to refresh token",
            ][..],
        ),
    ] {
        if phrases.iter().any(|phrase| output.contains(phrase)) {
            classes.insert(class);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn startup_diagnostic_drains_verbose_output_without_exposing_values() {
        let mut child = Command::new("sh")
            .args(["-c", "printf 'Error loading config: private-fixture-value\\n' >&2; head -c 100000 /dev/zero >&2"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let diagnostic = ProviderStartupDiagnostic::capture(&mut child);
        assert!(child.wait().unwrap().success());
        let deadline = Instant::now() + Duration::from_secs(2);
        while diagnostic.0.lock().unwrap().bytes < 100000 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let summary = diagnostic.summary();
        assert!(summary.contains("configuration"), "{summary}");
        assert!(!summary.contains("private-fixture-value"));
        assert!(diagnostic.0.lock().unwrap().bytes >= 100000);
    }

    #[test]
    fn bounded_auth_probe_summary_exposes_only_closed_classes() {
        let summary = summarize_provider_startup_output(
            b"bwrap: Creating new namespace failed: Operation not permitted: private-fixture-marker",
        );
        assert!(summary.contains("diagnostic_classes=namespace,permission_denied"));
        assert!(!summary.contains("private-fixture-marker"));
        assert!(!summary.contains("bwrap:"));
    }

    #[test]
    fn startup_diagnostic_unknown_output_has_no_provider_text() {
        let mut classes = BTreeSet::new();
        classify(
            b"private-fixture-value https://private.test/path",
            &mut classes,
        );
        assert!(classes.is_empty());
    }
}
