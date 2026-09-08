//! Real namespace acceptance plus bounded, credential-free child-process fixtures.
use std::fs::{self, DirBuilder, File};
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OUTPUT_LIMIT: u64 = 64 * 1024;

struct Probe {
    child: Child,
    root: PathBuf,
    deadline: Instant,
}

impl Probe {
    fn start(command: &mut Command, timeout: Duration) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-attachment-probe-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let child = command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(File::create(root.join("stdout")).unwrap())
            .stderr(File::create(root.join("stderr")).unwrap())
            .spawn();
        match child {
            Ok(child) => Self {
                child,
                root,
                deadline: Instant::now() + timeout,
            },
            Err(error) => {
                fs::remove_dir_all(&root).unwrap();
                panic!("spawn acceptance child: {error}");
            }
        }
    }

    fn output(&self, name: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        File::open(self.root.join(name))
            .map_err(|e| e.to_string())?
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(format!("{name} exceeded acceptance output limit"));
        }
        Ok(bytes)
    }

    fn stop(&mut self) {
        // The direct child is either an exec'd fixture or Bubblewrap, whose
        // --die-with-parent / PID namespace policy owns its sandbox children.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    fn poll(&mut self, readiness: bool) -> Result<Vec<u8>, String> {
        loop {
            let output = self.output("stdout");
            let diagnostic = self.output("stderr");
            let (output, diagnostic) = match (output, diagnostic) {
                (Ok(output), Ok(diagnostic)) => (output, diagnostic),
                (Err(error), _) | (_, Err(error)) => {
                    self.stop();
                    return Err(error);
                }
            };
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                if !readiness && status.success() {
                    return self.output("stdout");
                }
                return Err(format!(
                    "acceptance child exited {status}: {}",
                    String::from_utf8_lossy(&diagnostic)
                ));
            }
            if readiness && output == b"READY\n" {
                return Ok(output);
            }
            if Instant::now() >= self.deadline {
                self.stop();
                return Err("acceptance child deadline exceeded".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn stalled_readiness_is_bounded_and_reaped() {
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let mut probe = Probe::start(&mut command, Duration::from_millis(100));
    let root = probe.root.clone();
    let start = Instant::now();
    assert_eq!(
        probe.poll(true).unwrap_err(),
        "acceptance child deadline exceeded"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(probe.child.try_wait().unwrap().is_some());
    drop(probe);
    assert!(!root.exists());
}

#[test]
fn stalled_completion_is_bounded_and_reaped() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf 'READY\\n'; exec /bin/sleep 30"]);
    let mut probe = Probe::start(&mut command, Duration::from_millis(200));
    probe.poll(true).unwrap();
    assert_eq!(
        probe.poll(false).unwrap_err(),
        "acceptance child deadline exceeded"
    );
    assert!(probe.child.try_wait().unwrap().is_some());
}

#[test]
fn early_failure_is_not_reported_as_readiness() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf 'namespace denied' >&2; exit 42"]);
    let mut probe = Probe::start(&mut command, Duration::from_secs(2));
    let error = probe.poll(true).unwrap_err();
    assert!(error.contains("42"), "{error}");
    assert!(error.contains("namespace denied"), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a Linux host permitting the real managed Bubblewrap namespace"]
fn real_namespace_reads_late_attachment_read_only_and_hides_other_agent() {
    use super::{
        managed_namespace_args, managed_prompt_attachment_root, LaunchProviderRequest, BWRAP_PATH,
    };
    use crate::runtime::agent_actor::prompt_attachment_materialization::{
        inline_prompt_attachment_root, materialize_inline_prompt_attachments,
    };
    use base64::Engine;

    struct SessionRoot(PathBuf);
    impl Drop for SessionRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let session = format!(
        "attachment-acceptance-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let request = LaunchProviderRequest::new(&session, "codex", "codex", "default", "gpt-5.6-sol")
        .with_agent_id("selected");
    let own = inline_prompt_attachment_root(&session, "selected");
    let _cleanup = SessionRoot(own.parent().unwrap().to_path_buf());
    assert_eq!(
        managed_prompt_attachment_root(&request).unwrap(),
        Some(own.clone())
    );
    let other = inline_prompt_attachment_root(&session, "other");
    let (args, _) = managed_namespace_args(None, std::path::Path::exists, Some(&own));
    let mut command = Command::new(BWRAP_PATH);
    command.args(args).args(["--", "/bin/sh", "-c",
        "printf 'READY\\n'; while [ ! -f \"$1/.ready\" ]; do sleep 0.01; done; for file in \"$1\"/*; do if [ -f \"$file\" ]; then cat -- \"$file\" || exit 40; if printf x >>\"$file\" 2>/dev/null; then exit 41; fi; test ! -e \"$2\" || exit 42; printf 'CHECKS-OK\\n'; exit 0; fi; done; exit 43",
        "attachment-probe"]).arg(&own).arg(&other);
    let mut probe = Probe::start(&mut command, Duration::from_secs(10));
    probe.poll(true).unwrap();
    // Materialize the sibling first so it exists before the selected file lets
    // the child continue. This avoids a false isolation pass from timing.
    for (agent, contents) in [("other", "other agent"), ("selected", "late attachment\n")] {
        materialize_inline_prompt_attachments(
            &session,
            agent,
            vec![crate::session::PromptAttachment::new(
                "chariox-cloud://synthetic-acceptance",
                "text/plain",
                Some("probe.txt".into()),
            )
            .with_contents_base64(base64::engine::general_purpose::STANDARD.encode(contents))],
        )
        .unwrap();
    }
    fs::write(own.join(".ready"), b"").unwrap();
    assert_eq!(
        probe.poll(false).unwrap(),
        b"READY\nlate attachment\nCHECKS-OK\n"
    );
}
