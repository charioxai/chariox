//! Setup runs: the Project's script executed once per machine and checkout, with its output kept
//! under the field, a Stop that ends the process, and an honest result after a kernel restart.
use super::environment_error;
use crate::error::DaemonError;
use crate::local::{
    EnvironmentOperationKind, EnvironmentOperationState, EnvironmentOperationView,
    ProjectEnvironmentSetupState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use zeroize::Zeroizing;

const DIRECTORY: &str = "project-environment-setup";
/// The output a run keeps; older lines are dropped first.
const OUTPUT_TAIL_BYTES: usize = 64 * 1024;
/// How long a run's progress may sit only in memory.
const PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

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
    live: Mutex<HashMap<String, Arc<LiveRun>>>,
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
    pub(crate) fn new(private_state_root: &Path) -> Arc<Self> {
        Arc::new(Self {
            root: private_state_root.join(DIRECTORY),
            live: Mutex::new(HashMap::new()),
        })
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

    /// A run recorded as running with no live process was cut off by a kernel restart.
    fn settle_orphan(&self, mut run: StoredRun) -> Result<StoredRun, DaemonError> {
        if run.view.state == EnvironmentOperationState::Running
            && !lock(&self.live).contains_key(&run.view.operation_id)
        {
            run.view.state = EnvironmentOperationState::Failed;
            run.view.ended_at_ms = Some(crate::session::unix_epoch_ms());
            run.view.reason = Some("Kernel restarted".into());
            self.persist(&run)?;
        }
        Ok(run)
    }

    pub(crate) fn get(
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
        let Ok(last) = fs::read_to_string(self.last_path(project_id, checkout)) else {
            return Ok(ProjectEnvironmentSetupState::NotRun);
        };
        let Some(view) = self.get(last.trim())? else {
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
    pub(crate) fn start(
        self: &Arc<Self>,
        spec: SetupSpec,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        if let Some(existing) = self.get(&spec.operation_id)? {
            if existing.project_id != spec.project_id {
                return Err(environment_error(
                    "this operation id belongs to another Project",
                ));
            }
            return Ok(existing);
        }
        if let ProjectEnvironmentSetupState::Running { .. } =
            self.state_for(&spec.project_id, &spec.checkout, &spec.script_digest)?
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
            output_tail: String::new(),
        };
        let stored = StoredRun {
            view: view.clone(),
            script_digest: spec.script_digest.clone(),
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
        let mut command = crate::process_spawn::async_command("bash");
        command
            .arg("-c")
            .arg(&spec.script)
            .current_dir(&spec.checkout)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (name, value) in &spec.environment {
            command.env(name, value.as_str());
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Setup could not start bash".to_string())?;
        *lock(&live.pid) = child.id();
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
            .wait()
            .await
            .map_err(|_| "Setup could not be awaited".to_string());
        for reader in readers.into_iter().flatten() {
            let _ = reader.await;
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
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            let mut persisted = std::time::Instant::now();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut line = line;
                for (name, value) in secrets.iter() {
                    if !value.is_empty() {
                        line = line.replace(value.as_str(), &format!("[{name}]"));
                    }
                }
                let snapshot = {
                    let mut run = lock(&live.run);
                    append_tail(&mut run.view.output_tail, &line);
                    (persisted.elapsed() >= PERSIST_INTERVAL).then(|| run.clone())
                };
                if let Some(snapshot) = snapshot {
                    persisted = std::time::Instant::now();
                    let _ = write_run(&runs_root, &snapshot);
                }
            }
        })
    }

    fn finish(&self, live: &Arc<LiveRun>, outcome: Result<std::process::ExitStatus, String>) {
        let mut run = lock(&live.run);
        run.view.ended_at_ms = Some(crate::session::unix_epoch_ms());
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

fn append_tail(tail: &mut String, line: &str) {
    tail.push_str(line);
    tail.push('\n');
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
        let runs = SetupRuns::new(&dir.0);
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
        let runs = SetupRuns::new(&dir.0);
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
        let runs = SetupRuns::new(&dir.0);
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

    #[tokio::test]
    async fn a_stop_sent_before_the_process_exists_still_ends_the_run() {
        let dir = scratch();
        let runs = SetupRuns::new(&dir.0);
        runs.start(spec(&dir, "op-early", "sleep 60")).unwrap();
        runs.cancel("op-early").unwrap();
        let view = finished(&runs, "op-early").await;
        assert_eq!(view.state, EnvironmentOperationState::Cancelled);
    }

    #[tokio::test]
    async fn a_run_cut_off_by_a_kernel_restart_reads_as_failed_with_that_reason() {
        let dir = scratch();
        let before = SetupRuns::new(&dir.0);
        before.start(spec(&dir, "op-restart", "sleep 60")).unwrap();
        let live = lock(&before.live).get("op-restart").cloned().unwrap();
        // The new kernel process has no live run for the persisted one.
        let after = SetupRuns::new(&dir.0);
        let view = after.get("op-restart").unwrap().unwrap();
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert_eq!(view.reason.as_deref(), Some("Kernel restarted"));
        assert!(matches!(
            after
                .state_for("project", &dir.0.join("checkout"), &digest(&["sleep 60"]))
                .unwrap(),
            ProjectEnvironmentSetupState::Failed { .. }
        ));
        before.cancel("op-restart").unwrap();
        drop(live);
    }

    #[test]
    fn the_output_tail_drops_the_oldest_text_on_a_character_boundary() {
        let mut tail = String::new();
        for _ in 0..OUTPUT_TAIL_BYTES {
            append_tail(&mut tail, "é");
        }
        assert!(tail.len() <= OUTPUT_TAIL_BYTES);
        assert!(tail.ends_with("é\n"));
    }
}
