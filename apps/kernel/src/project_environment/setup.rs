//! Setup runs: the Project's script executed once per machine and checkout, with its output kept
//! under the field, a Stop that ends the process, and an honest result after a kernel restart:
//! a run the previous kernel left behind stays running until its processes are proven gone.
use super::environment_error;
use crate::error::DaemonError;
use crate::local::{
    EnvironmentOperationKind, EnvironmentOperationState, EnvironmentOperationView,
    ProjectEnvironmentSetupState,
};
use crate::runtime::bounded_process::AsyncChildOwner as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

const DIRECTORY: &str = "project-environment-setup";
/// The output a run keeps; the oldest text is dropped first.
const OUTPUT_TAIL_BYTES: usize = 64 * 1024;
/// One read from the script's output: text is shown as it arrives, newline or not.
const OUTPUT_READ_BYTES: usize = 8 * 1024;
/// How long output may still arrive after the script exited.
const OUTPUT_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long a run's progress may sit only in memory.
const OUTPUT_ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const RECONCILING: &str = "Kernel restarted; stopping the Setup it left running";

pub(crate) struct SetupSpec {
    pub operation_id: String,
    pub project_id: String,
    pub checkout: PathBuf,
    pub script: String,
    pub script_digest: String,
    /// Variables and Secret values for the script's environment.
    pub environment: Vec<(String, Zeroizing<String>)>,
    /// Secret names and values, so output never shows a value.
    pub secrets: Vec<(String, Zeroizing<String>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRun {
    view: EnvironmentOperationView,
    script_digest: String,
    /// Saved before the script starts and inherited by all of its processes, so a later kernel
    /// finds them even when the previous one died before saving the leader.
    claim: String,
    /// The script's session leader: its PID and birth.
    leader: Option<(u32, String)>,
}

struct LiveRun {
    run: Mutex<StoredRun>,
    pid: Mutex<Option<u32>>,
    cancelled: std::sync::atomic::AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct SetupRuns {
    root: PathBuf,
    changes: super::OperationChanges,
    /// Held while a start checks, saves and registers its run, while a run publishes its end, and
    /// while a reader decides a saved run is an orphan: no start is admitted twice, and no reader
    /// sees a saved run before it is live or after it was removed from `live` but not yet ended.
    publication: Mutex<()>,
    live: Mutex<HashMap<String, Arc<LiveRun>>>,
    managed: crate::room_terminal::managed_work::ManagedWork,
}

fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

impl SetupRuns {
    /// Starts stopping whatever the previous kernel's runs left running.
    pub(crate) fn new(private_state_root: &Path, changes: super::OperationChanges) -> Arc<Self> {
        let runs = Arc::new(Self {
            root: private_state_root.join(DIRECTORY),
            changes,
            publication: Mutex::new(()),
            live: Mutex::new(HashMap::new()),
            managed: Default::default(),
        });
        for entry in fs::read_dir(&runs.root).into_iter().flatten().flatten() {
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(mut run) = serde_json::from_slice::<StoredRun>(&bytes) else {
                continue;
            };
            // A signalled tree is usually gone within moments; a slower one keeps its run open.
            for _ in 0..40 {
                match runs.settle_orphan(run) {
                    Ok(settled) if settled.view.state == EnvironmentOperationState::Running => {
                        run = settled;
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    _ => break,
                }
            }
        }
        runs
    }

    fn run_path(&self, operation_id: &str) -> PathBuf {
        self.root
            .join(format!("run-{}.json", digest(&[operation_id])))
    }

    /// The newest run of one checkout, which decides its Setup state.
    fn last_path(&self, project_id: &str, checkout: &Path) -> PathBuf {
        self.root.join(format!(
            "last-{}",
            digest(&[project_id, &checkout.to_string_lossy()])
        ))
    }

    fn persist(&self, run: &StoredRun) -> Result<(), DaemonError> {
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("the Setup output directory is unavailable"))?;
        let bytes = serde_json::to_vec(run)
            .map_err(|_| environment_error("the Setup run could not be encoded"))?;
        let temporary = self.root.join(format!(".tmp-{}", rand::random::<u64>()));
        let result = fs::write(&temporary, bytes)
            .and_then(|_| fs::rename(&temporary, self.run_path(&run.view.operation_id)));
        let _ = fs::remove_file(&temporary);
        result.map_err(|_| environment_error("saving the Setup run failed"))
    }

    fn load(&self, operation_id: &str) -> Result<Option<StoredRun>, DaemonError> {
        match fs::read(self.run_path(operation_id)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| environment_error("the saved Setup run is not valid")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(environment_error("the saved Setup run is unreadable")),
        }
    }

    /// A run recorded as running with no live process was cut off by a kernel restart. It ends
    /// as failed only once none of its processes remain; until then it keeps the checkout busy.
    fn settle_orphan(&self, mut run: StoredRun) -> Result<StoredRun, DaemonError> {
        if run.view.state != EnvironmentOperationState::Running
            || lock(&self.live).contains_key(&run.view.operation_id)
        {
            return Ok(run);
        }
        let leader = run
            .leader
            .as_ref()
            .map(|(pid, birth)| (*pid, birth.as_str()));
        match crate::runtime::owned_process_signals::OwnedProcessSignals::end_claimed_tree(
            &run.claim, leader,
        ) {
            Ok(true) => {
                run.view.state = EnvironmentOperationState::Failed;
                run.view.ended_at_ms = Some(crate::session::unix_epoch_ms());
                run.view.reason = Some("Kernel restarted".into());
            }
            _ if run.view.reason.as_deref() == Some(RECONCILING) => return Ok(run),
            _ => run.view.reason = Some(RECONCILING.into()),
        }
        run.view.updates += 1;
        self.persist(&run)?;
        self.changes.notify();
        Ok(run)
    }

    pub(crate) fn get(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        let _publication = lock(&self.publication);
        self.published(operation_id)
    }

    fn published(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        if let Some(live) = lock(&self.live).get(operation_id) {
            return Ok(Some(lock(&live.run).view.clone()));
        }
        self.load(operation_id)?
            .map(|run| self.settle_orphan(run).map(|run| run.view))
            .transpose()
    }

    pub(crate) fn state_for(
        &self,
        project_id: &str,
        checkout: &Path,
        script_digest: &str,
    ) -> Result<ProjectEnvironmentSetupState, DaemonError> {
        let _publication = lock(&self.publication);
        self.published_state(project_id, checkout, script_digest)
    }

    fn published_state(
        &self,
        project_id: &str,
        checkout: &Path,
        script_digest: &str,
    ) -> Result<ProjectEnvironmentSetupState, DaemonError> {
        let Ok(last) = fs::read_to_string(self.last_path(project_id, checkout)) else {
            return Ok(ProjectEnvironmentSetupState::NotRun);
        };
        let Some(view) = self.published(last.trim())? else {
            return Ok(ProjectEnvironmentSetupState::NotRun);
        };
        let digest_matches = match lock(&self.live).get(&view.operation_id) {
            Some(live) => lock(&live.run).script_digest == script_digest,
            None => self
                .load(&view.operation_id)?
                .is_some_and(|run| run.script_digest == script_digest),
        };
        let ended_at_ms = view.ended_at_ms.unwrap_or(view.started_at_ms);
        let operation_id = view.operation_id;
        Ok(match view.state {
            EnvironmentOperationState::Running => ProjectEnvironmentSetupState::Running {
                operation_id,
                started_at_ms: view.started_at_ms,
            },
            _ if !digest_matches => ProjectEnvironmentSetupState::NotRun,
            EnvironmentOperationState::Succeeded => ProjectEnvironmentSetupState::Ok {
                operation_id,
                ended_at_ms,
            },
            EnvironmentOperationState::Failed => ProjectEnvironmentSetupState::Failed {
                operation_id,
                ended_at_ms,
                exit_code: view.exit_code,
                reason: view.reason,
            },
            EnvironmentOperationState::Cancelled => ProjectEnvironmentSetupState::Cancelled {
                operation_id,
                ended_at_ms,
            },
        })
    }

    /// Starts the script, or answers the run that already carries this operation id.
    pub(crate) fn link_managed_kernel(
        &self,
        gate: Arc<crate::runtime::state::ManagedKernelQuiescenceGate>,
        callback: std::sync::Weak<dyn Fn() + Send + Sync>,
    ) {
        self.managed.link(gate, callback);
    }

    pub(crate) fn has_running_work(&self) -> bool {
        // Keep cancelled runs counted until the child is reaped and its output has drained.
        !lock(&self.live).is_empty()
    }

    fn report_work(&self) {
        self.managed.report(|| self.has_running_work());
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        spec: SetupSpec,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        let _admission = self.managed.admission()?;
        // An ordinary kernel has no managed admission; a retry of a lost reply races the start.
        let _publication = lock(&self.publication);
        if let Some(existing) = self.published(&spec.operation_id)? {
            if existing.project_id != spec.project_id {
                return Err(environment_error(
                    "this operation id belongs to another Project",
                ));
            }
            return Ok(existing);
        }
        if let ProjectEnvironmentSetupState::Running { .. } =
            self.published_state(&spec.project_id, &spec.checkout, &spec.script_digest)?
        {
            return Err(environment_error("Setup is already running here"));
        }
        let view = EnvironmentOperationView {
            operation_id: spec.operation_id.clone(),
            project_id: spec.project_id.clone(),
            checkout_path: spec.checkout.to_string_lossy().into_owned(),
            kind: EnvironmentOperationKind::Setup,
            state: EnvironmentOperationState::Running,
            started_at_ms: crate::session::unix_epoch_ms(),
            ended_at_ms: None,
            exit_code: None,
            reason: None,
            updates: 0,
            output_tail: String::new(),
            entries: Vec::new(),
            source_label: None,
        };
        let stored = StoredRun {
            view: view.clone(),
            script_digest: spec.script_digest.clone(),
            claim: format!("{:032x}", rand::random::<u128>()),
            leader: None,
        };
        self.persist(&stored)?;
        fs::write(
            self.last_path(&spec.project_id, &spec.checkout),
            &spec.operation_id,
        )
        .map_err(|_| environment_error("saving the Setup run failed"))?;
        let live = Arc::new(LiveRun {
            run: Mutex::new(stored),
            pid: Mutex::new(None),
            cancelled: std::sync::atomic::AtomicBool::new(false),
        });
        lock(&self.live).insert(spec.operation_id.clone(), Arc::clone(&live));
        self.report_work();
        let runs = Arc::clone(self);
        tokio::spawn(async move {
            let outcome = runs.execute(&live, &spec).await;
            runs.finish(&live, outcome);
        });
        Ok(view)
    }

    async fn execute(
        &self,
        live: &Arc<LiveRun>,
        spec: &SetupSpec,
    ) -> Result<std::process::ExitStatus, String> {
        let claim = lock(&live.run).claim.clone();
        let mut command = crate::process_spawn::async_command("bash");
        command
            .env(
                crate::runtime::owned_process_signals::CLAIM_VARIABLE,
                &claim,
            )
            .arg("-c")
            .arg(&spec.script)
            .current_dir(&spec.checkout)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (name, value) in &spec.environment {
            command.env(name, value.as_str());
        }
        // Its own session, so a later kernel can tell the script's processes from others.
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Setup could not start bash".to_string())?;
        *lock(&live.pid) = child.id();
        let leader =
            crate::runtime::owned_process_signals::OwnedProcessSignals::for_tokio_session_child(
                &child,
            )
            .map(|signals| signals.root_birth());
        let saved = leader.ok().and_then(|leader| {
            let mut run = lock(&live.run);
            run.leader = Some(leader);
            self.persist(&run).ok()
        });
        if saved.is_none() {
            if let Some(pid) = child.id() {
                crate::runtime::process_health::terminate_process_tree(pid);
            }
            let _ = child.supervised_wait().await;
            return Err("Setup could not save its process".to_string());
        }
        // A Stop that arrived before the process existed found no PID to end.
        if live.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(pid) = child.id() {
                crate::runtime::process_health::terminate_process_tree(pid);
            }
        }
        let secrets: Arc<Vec<(String, Zeroizing<String>)>> = Arc::new(spec.secrets.clone());
        let readers = [
            child
                .stdout
                .take()
                .map(|pipe| self.pump(live, pipe, &secrets)),
            child
                .stderr
                .take()
                .map(|pipe| self.pump(live, pipe, &secrets)),
        ];
        let status = child
            .supervised_wait()
            .await
            .map_err(|_| "Setup could not be awaited".to_string());
        // A script that leaves a background process holding the pipes must not keep the run open.
        let drain_until = tokio::time::Instant::now() + OUTPUT_DRAIN_GRACE;
        for reader in readers.into_iter().flatten() {
            let abort = reader.abort_handle();
            if tokio::time::timeout_at(drain_until, reader).await.is_err() {
                abort.abort();
            }
        }
        status
    }

    fn pump(
        &self,
        live: &Arc<LiveRun>,
        pipe: impl AsyncRead + Unpin + Send + 'static,
        secrets: &Arc<Vec<(String, Zeroizing<String>)>>,
    ) -> tokio::task::JoinHandle<()> {
        let live = Arc::clone(live);
        let secrets = Arc::clone(secrets);
        let runs_root = self.root.clone();
        let changes = self.changes.clone();
        tokio::spawn(async move {
            let mut pipe = pipe;
            let mut read = vec![0; OUTPUT_READ_BYTES];
            let mut output = OutputText::new(secrets);
            let mut persisted = std::time::Instant::now();
            let mut announced = std::time::Instant::now();
            let announce_pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
            loop {
                let (text, ended) = match pipe.read(&mut read).await {
                    Ok(0) | Err(_) => (output.finish(), true),
                    Ok(count) => (output.push(&read[..count]), false),
                };
                if text.is_empty() {
                    if ended {
                        break;
                    }
                    continue;
                }
                let snapshot = {
                    let mut run = lock(&live.run);
                    append_tail(&mut run.view.output_tail, &text);
                    run.view.updates += 1;
                    (persisted.elapsed() >= PERSIST_INTERVAL).then(|| run.clone())
                };
                // A burst of output wakes readers at most every OUTPUT_ANNOUNCE_INTERVAL; a delayed wake
                // announces the end of a burst, so output never waits for the next read to show.
                if announced.elapsed() >= OUTPUT_ANNOUNCE_INTERVAL {
                    announced = std::time::Instant::now();
                    changes.notify();
                } else if !announce_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let changes = changes.clone();
                    let announce_pending = Arc::clone(&announce_pending);
                    tokio::spawn(async move {
                        tokio::time::sleep(OUTPUT_ANNOUNCE_INTERVAL).await;
                        announce_pending.store(false, std::sync::atomic::Ordering::SeqCst);
                        changes.notify();
                    });
                }
                if let Some(snapshot) = snapshot {
                    persisted = std::time::Instant::now();
                    let _ = write_run(&runs_root, &snapshot);
                }
                if ended {
                    break;
                }
            }
        })
    }

    fn finish(&self, live: &Arc<LiveRun>, outcome: Result<std::process::ExitStatus, String>) {
        let publication = lock(&self.publication);
        let mut run = lock(&live.run);
        run.view.ended_at_ms = Some(crate::session::unix_epoch_ms());
        run.view.updates += 1;
        if live.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            run.view.state = EnvironmentOperationState::Cancelled;
        } else {
            match outcome {
                Ok(status) if status.success() => {
                    run.view.state = EnvironmentOperationState::Succeeded
                }
                Ok(status) => {
                    run.view.state = EnvironmentOperationState::Failed;
                    run.view.exit_code = status.code();
                    if status.code().is_none() {
                        run.view.reason = Some("Setup was ended by a signal".into());
                    }
                }
                Err(reason) => {
                    run.view.state = EnvironmentOperationState::Failed;
                    run.view.reason = Some(reason);
                }
            }
        }
        let _ = self.persist(&run);
        let operation_id = run.view.operation_id.clone();
        drop(run);
        lock(&self.live).remove(&operation_id);
        drop(publication);
        self.report_work();
        self.changes.notify();
    }

    /// Ends the run's process tree by its exact PID; the run then settles as cancelled.
    pub(crate) fn cancel(
        &self,
        operation_id: &str,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        let live = lock(&self.live).get(operation_id).cloned();
        if let Some(live) = live {
            live.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(pid) = *lock(&live.pid) {
                crate::runtime::process_health::terminate_process_tree(pid);
            }
            return Ok(lock(&live.run).view.clone());
        }
        self.get(operation_id)?
            .ok_or_else(|| environment_error("this Setup run is unknown"))
    }
}

fn write_run(root: &Path, run: &StoredRun) -> std::io::Result<()> {
    let path = root.join(format!("run-{}.json", digest(&[&run.view.operation_id])));
    let temporary = root.join(format!(".tmp-{}", rand::random::<u64>()));
    let bytes = serde_json::to_vec(run).map_err(std::io::Error::other)?;
    let result = fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, path));
    let _ = fs::remove_file(&temporary);
    result
}

/// A script's output between reads: bytes that end inside a UTF-8 character, and text that ends
/// with the start of a Secret value the next read may complete. Both are bounded, so a script
/// without newlines cannot grow the kernel's memory.
struct OutputText {
    secrets: Arc<Vec<(String, Zeroizing<String>)>>,
    bytes: Vec<u8>,
    text: String,
}

impl OutputText {
    fn new(secrets: Arc<Vec<(String, Zeroizing<String>)>>) -> Self {
        Self {
            secrets,
            bytes: Vec::new(),
            text: String::new(),
        }
    }

    /// Takes one read and returns the redacted text that can be shown now.
    fn push(&mut self, read: &[u8]) -> String {
        self.bytes.extend_from_slice(read);
        let mut start = 0;
        loop {
            match std::str::from_utf8(&self.bytes[start..]) {
                Ok(text) => {
                    self.text.push_str(text);
                    start = self.bytes.len();
                    break;
                }
                Err(error) => {
                    let valid = start + error.valid_up_to();
                    self.text.push_str(
                        std::str::from_utf8(&self.bytes[start..valid]).unwrap_or_default(),
                    );
                    let Some(invalid) = error.error_len() else {
                        start = valid;
                        break;
                    };
                    self.text.push(char::REPLACEMENT_CHARACTER);
                    start = valid + invalid;
                }
            }
        }
        self.bytes.drain(..start);
        self.shown(false)
    }

    /// Returns everything still held once the output has ended.
    fn finish(&mut self) -> String {
        let bytes = std::mem::take(&mut self.bytes);
        self.text.push_str(&String::from_utf8_lossy(&bytes));
        self.shown(true)
    }

    fn shown(&mut self, all: bool) -> String {
        for (name, value) in self.secrets.iter() {
            if !value.is_empty() && self.text.contains(value.as_str()) {
                self.text = self.text.replace(value.as_str(), &format!("[{name}]"));
            }
        }
        let held = if all { 0 } else { self.secret_start() };
        let rest = self.text.split_off(self.text.len() - held);
        std::mem::replace(&mut self.text, rest)
    }

    /// The longest end of the text that begins a Secret value.
    fn secret_start(&self) -> usize {
        self.secrets
            .iter()
            .filter_map(|(_, value)| {
                (1..value.len())
                    .rev()
                    .filter(|length| value.is_char_boundary(*length))
                    .find(|length| self.text.ends_with(&value[..*length]))
            })
            .max()
            .unwrap_or(0)
    }
}

fn append_tail(tail: &mut String, text: &str) {
    tail.push_str(text);
    if tail.len() > OUTPUT_TAIL_BYTES {
        let mut cut = tail.len() - OUTPUT_TAIL_BYTES;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("chariox-setup-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(dir.join("checkout")).unwrap();
        Scratch(dir)
    }
    fn spec(scratch: &Scratch, operation_id: &str, script: &str) -> SetupSpec {
        SetupSpec {
            operation_id: operation_id.into(),
            project_id: "project".into(),
            checkout: scratch.0.join("checkout"),
            script: script.into(),
            script_digest: digest(&[script]),
            environment: vec![
                ("LOG_LEVEL".into(), Zeroizing::new("debug".into())),
                (
                    "NOTION_TOKEN".into(),
                    Zeroizing::new("synthetic-token-value".into()),
                ),
            ],
            secrets: vec![(
                "NOTION_TOKEN".into(),
                Zeroizing::new("synthetic-token-value".into()),
            )],
        }
    }
    async fn finished(runs: &Arc<SetupRuns>, operation_id: &str) -> EnvironmentOperationView {
        for _ in 0..200 {
            let view = runs.get(operation_id).unwrap().unwrap();
            if view.state != EnvironmentOperationState::Running {
                return view;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the Setup run did not finish");
    }

    #[tokio::test]
    async fn a_run_executes_in_the_checkout_with_the_environment_and_never_prints_a_secret_value() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        let script = "pwd; echo level=$LOG_LEVEL; echo token=$NOTION_TOKEN";
        runs.start(spec(&dir, "op-1", script)).unwrap();
        let view = finished(&runs, "op-1").await;
        assert_eq!(view.state, EnvironmentOperationState::Succeeded);
        assert!(
            view.output_tail.contains("checkout"),
            "{}",
            view.output_tail
        );
        assert!(view.output_tail.contains("level=debug"));
        assert!(view.output_tail.contains("token=[NOTION_TOKEN]"));
        assert!(!view.output_tail.contains("synthetic-token-value"));
        let persisted = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap_or_default())
            .collect::<String>();
        assert!(!persisted.contains("synthetic-token-value"));
        let state = runs
            .state_for("project", &dir.0.join("checkout"), &digest(&[script]))
            .unwrap();
        assert!(matches!(state, ProjectEnvironmentSetupState::Ok { .. }));
    }

    #[tokio::test]
    async fn a_changed_script_makes_the_last_run_not_run_and_a_failed_exit_is_reported() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        runs.start(spec(&dir, "op-fail", "echo before; exit 3"))
            .unwrap();
        let view = finished(&runs, "op-fail").await;
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert_eq!(view.exit_code, Some(3));
        let checkout = dir.0.join("checkout");
        assert!(matches!(
            runs.state_for("project", &checkout, &digest(&["echo before; exit 3"]))
                .unwrap(),
            ProjectEnvironmentSetupState::Failed {
                exit_code: Some(3),
                ..
            }
        ));
        assert!(matches!(
            runs.state_for("project", &checkout, &digest(&["echo changed"]))
                .unwrap(),
            ProjectEnvironmentSetupState::NotRun
        ));
    }

    #[tokio::test]
    async fn stop_ends_the_process_by_its_exact_pid_and_the_run_settles_as_cancelled() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        runs.start(spec(&dir, "op-stop", "echo started; sleep 60"))
            .unwrap();
        let live = lock(&runs.live).get("op-stop").cloned().unwrap();
        let pid = loop {
            if let Some(pid) = *lock(&live.pid) {
                break pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        assert!(crate::runtime::process_health::process_running(pid));
        assert!(
            runs.start(spec(&dir, "op-second", "echo second")).is_err(),
            "one run per checkout"
        );
        runs.cancel("op-stop").unwrap();
        let view = finished(&runs, "op-stop").await;
        assert_eq!(view.state, EnvironmentOperationState::Cancelled);
        assert!(view.output_tail.contains("started"));
        for _ in 0..100 {
            if !crate::runtime::process_health::process_running(pid) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the Setup process kept running after Stop");
    }

    /// Output without a newline is shown while the script still runs, the kept text stays
    /// bounded, and a Secret value split across two reads is still hidden.
    #[tokio::test]
    async fn output_without_a_newline_is_bounded_shown_while_running_and_redacted_across_reads() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        let script = "head -c 1048576 /dev/zero | tr '\\0' x; printf ' token=synthetic-tok'; \
                      sleep 0.3; printf 'en-value\\rprogress 50%%'; sleep 60";
        runs.start(spec(&dir, "op-unbroken", script)).unwrap();
        let mut view = runs.get("op-unbroken").unwrap().unwrap();
        for _ in 0..200 {
            if view.output_tail.ends_with("progress 50%") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            view = runs.get("op-unbroken").unwrap().unwrap();
        }
        assert_eq!(view.state, EnvironmentOperationState::Running);
        assert!(
            view.output_tail
                .ends_with(" token=[NOTION_TOKEN]\rprogress 50%"),
            "{:?}",
            &view.output_tail[view.output_tail.len().saturating_sub(80)..]
        );
        assert!(view.output_tail.len() <= OUTPUT_TAIL_BYTES);
        runs.cancel("op-unbroken").unwrap();
        let view = finished(&runs, "op-unbroken").await;
        assert!(!view.output_tail.contains("synthetic-tok"));
        let persisted = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap_or_default())
            .collect::<String>();
        assert!(!persisted.contains("synthetic-tok"));
    }

    /// An ordinary kernel has no managed admission: a lost-reply retry with the same id and a
    /// start for another id race the first start, and a reader polls the run meanwhile.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_starts_admit_one_run_that_stop_ends_and_readers_never_see_as_an_orphan() {
        for _ in 0..20 {
            let dir = scratch();
            let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
            let script = "echo $$ >> ../executions; sleep 30";
            let ids = ["op-same", "op-same", "op-same", "op-other"];
            let barrier = Arc::new(std::sync::Barrier::new(ids.len() + 1));
            let starts: Vec<_> = ids
                .iter()
                .map(|id| {
                    let (runs, barrier) = (Arc::clone(&runs), Arc::clone(&barrier));
                    let (spec, runtime) =
                        (spec(&dir, id, script), tokio::runtime::Handle::current());
                    std::thread::spawn(move || {
                        let _runtime = runtime.enter();
                        barrier.wait();
                        runs.start(spec).ok()
                    })
                })
                .collect();
            let reader = {
                let (runs, barrier) = (Arc::clone(&runs), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    (0..2_000)
                        .filter_map(|_| runs.get("op-same").unwrap())
                        .all(|view| view.state == EnvironmentOperationState::Running)
                })
            };
            let started: Vec<_> = starts
                .into_iter()
                .map(|start| start.join().unwrap())
                .collect();
            assert!(
                reader.join().unwrap(),
                "a reader settled an admitted run as an orphan"
            );
            let admitted: std::collections::HashSet<_> = started
                .iter()
                .flatten()
                .map(|view| (view.operation_id.clone(), view.started_at_ms))
                .collect();
            assert_eq!(admitted.len(), 1, "one run per checkout: {admitted:?}");
            let (operation_id, _) = admitted.into_iter().next().unwrap();
            while !dir.0.join("executions").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let pids: Vec<u32> = fs::read_to_string(dir.0.join("executions"))
                .unwrap()
                .lines()
                .map(|pid| pid.parse().unwrap())
                .collect();
            runs.cancel(&operation_id).unwrap();
            let view = finished(&runs, &operation_id).await;
            assert_eq!(view.state, EnvironmentOperationState::Cancelled);
            for _ in 0..100 {
                if !pids
                    .iter()
                    .any(|pid| crate::runtime::process_health::process_running(*pid))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            assert!(
                !pids
                    .iter()
                    .any(|pid| crate::runtime::process_health::process_running(*pid)),
                "Stop missed a Setup process: {pids:?}"
            );
            assert_eq!(pids.len(), 1, "the script ran more than once: {pids:?}");
            assert!(!runs.has_running_work());
        }
    }

    #[tokio::test]
    async fn a_background_process_holding_the_output_does_not_keep_the_run_open() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        runs.start(spec(&dir, "op-background", "sleep 4 & echo done"))
            .unwrap();
        let view = finished(&runs, "op-background").await;
        assert_eq!(view.state, EnvironmentOperationState::Succeeded);
        assert!(view.output_tail.contains("done"));
        assert!(
            view.ended_at_ms.unwrap() - view.started_at_ms < 3_500,
            "the run waited for the background process"
        );
    }

    #[tokio::test]
    async fn a_stop_sent_before_the_process_exists_still_ends_the_run() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        runs.start(spec(&dir, "op-early", "sleep 60")).unwrap();
        runs.cancel("op-early").unwrap();
        let view = finished(&runs, "op-early").await;
        assert_eq!(view.state, EnvironmentOperationState::Cancelled);
    }

    /// What a kernel that died this moment leaves: a saved running run, and its script.
    fn orphan(
        dir: &Scratch,
        operation_id: &str,
        script: &str,
        leader: bool,
    ) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let claim = format!("{:032x}", rand::random::<u128>());
        let mut command = std::process::Command::new("bash");
        command
            .args(["-c", script])
            .current_dir(dir.0.join("checkout"))
            .env(
                crate::runtime::owned_process_signals::CLAIM_VARIABLE,
                &claim,
            );
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        let mut view = runs.start(spec(dir, operation_id, "true")).unwrap();
        while runs.get(operation_id).unwrap().unwrap().state == EnvironmentOperationState::Running {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        view.state = EnvironmentOperationState::Running;
        let birth = crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child)
            .unwrap()
            .root_birth();
        runs.persist(&StoredRun {
            view,
            script_digest: digest(&["true"]),
            claim,
            leader: leader.then_some(birth),
        })
        .unwrap();
        child
    }

    fn exited(child: &mut std::process::Child) -> bool {
        (0..200).any(|_| {
            let done = child.try_wait().unwrap().is_some();
            std::thread::sleep(std::time::Duration::from_millis(10));
            done
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_kernel_ends_the_script_the_previous_one_left_before_admitting_another() {
        let dir = scratch();
        // The descendant writes the marker; the leader exits, leaving it reparented.
        let mut script = orphan(
            &dir,
            "op-restart",
            "(sleep 1; touch finished) & echo $! > child.pid; wait",
            true,
        );
        while !dir.0.join("checkout/child.pid").exists() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let after = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        assert!(
            exited(&mut script),
            "the previous kernel's script kept running"
        );
        let view = after.get("op-restart").unwrap().unwrap();
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert_eq!(view.reason.as_deref(), Some("Kernel restarted"));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(
            !dir.0.join("checkout/finished").exists(),
            "a descendant of the previous kernel's script ran on"
        );
        after.start(spec(&dir, "op-next", "echo next")).unwrap();
        assert_eq!(
            finished(&after, "op-next").await.state,
            EnvironmentOperationState::Succeeded
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_started_before_its_leader_was_saved_is_still_found_and_ended() {
        let dir = scratch();
        let mut script = orphan(&dir, "op-claim", "sleep 30", false);
        let after = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        assert!(exited(&mut script), "an unsaved leader kept running");
        assert_eq!(
            after.get("op-claim").unwrap().unwrap().state,
            EnvironmentOperationState::Failed
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reused_leader_pid_is_neither_signalled_nor_keeps_the_checkout_busy() {
        let dir = scratch();
        let mut script = orphan(&dir, "op-reused", "true", false);
        script.wait().unwrap();
        let mut run = SetupRuns::new(&dir.0, super::super::OperationChanges::new())
            .load("op-reused")
            .unwrap()
            .unwrap();
        // The saved leader PID now names this test process, with another birth.
        run.view.state = EnvironmentOperationState::Running;
        run.leader = Some((std::process::id(), "earlier-birth".into()));
        let runs = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        runs.persist(&run).unwrap();
        let after = SetupRuns::new(&dir.0, super::super::OperationChanges::new());
        assert_eq!(
            after.get("op-reused").unwrap().unwrap().state,
            EnvironmentOperationState::Failed
        );
    }

    #[test]
    fn the_output_tail_drops_the_oldest_text_on_a_character_boundary() {
        let mut tail = String::new();
        for _ in 0..OUTPUT_TAIL_BYTES {
            append_tail(&mut tail, "é");
        }
        assert!(tail.len() <= OUTPUT_TAIL_BYTES);
        assert!(tail.ends_with('é'));
    }
}
