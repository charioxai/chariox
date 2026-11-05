//! A disk running out of room (OF-39, OF-40): idle temp folders are trimmed
//! first; if the volume stays below its floor, the agents holding the most are
//! asked to free what they can rebuild, naming what each holds. The kernel
//! deletes nothing but its own scratch and decides nothing about content.
use super::*;
use crate::runtime::process_resources::{self, SessionProcess, SessionProcessOwner};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

const CHECK_EVERY: Duration = Duration::from_secs(5);
const TRIM_EVERY: Duration = Duration::from_secs(60);
/// Under pressure an item nothing wrote for this long goes, however long the
/// usual window is.
const PRESSURE_IDLE: Duration = Duration::from_secs(60);
const ASK_TIMEOUT: Duration = Duration::from_secs(90);
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(600);
/// Agents asked at once, the biggest holders first.
const ASKED_AT_ONCE: usize = 3;
/// Below this an agent holds too little for an ask to matter.
const WORTH_ASKING: u64 = 64 * 1024 * 1024;
const FOLDERS_NAMED: usize = 5;

#[derive(Default)]
struct Pace {
    trimmed: Option<Instant>,
    asked: BTreeMap<(String, String), Instant>,
}

/// A blocking syscall already in progress may finish after shutdown; it must
/// not begin another scan or trim once the owning maintenance task is gone.
struct ScanCancellation(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ScanCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct PendingDiskPrompt<'a> {
    state: &'a KernelRuntimeState,
    session: &'a str,
    prompt: Option<String>,
}

impl Drop for PendingDiskPrompt<'_> {
    fn drop(&mut self) {
        if let Some(prompt) = self.prompt.take() {
            let result = self
                .state
                .owned
                .durable_state_store
                .agent_task(&prompt)
                .and_then(|task| {
                    if task.is_some() {
                        self.state.owned.withdraw_agent_task(&prompt)
                    } else {
                        Ok(())
                    }
                });
            if let Err(error) = result {
                tracing::warn!(%error, "could not withdraw cancelled disk-pressure admission");
            }
            self.state.publish_task_change(self.session);
        }
    }
}

fn due(slot: &mut Option<Instant>, every: Duration) -> bool {
    if slot.is_some_and(|at| at.elapsed() < every) {
        return false;
    }
    *slot = Some(Instant::now());
    true
}

/// What one agent holds on disk: its live processes (and what they keep open),
/// its temp folder and the biggest folders of its workspace.
pub(crate) struct Holding {
    pub session_id: String,
    pub agent_id: String,
    pub processes: Vec<SessionProcess>,
    pub temp: Option<PathBuf>,
    pub temp_bytes: u64,
    pub temp_biggest: Vec<(PathBuf, u64)>,
    pub workspace: Option<PathBuf>,
    pub workspace_biggest: Vec<(PathBuf, u64)>,
}

impl Holding {
    pub(crate) fn used(&self) -> u64 {
        let processes: u64 = self
            .processes
            .iter()
            .map(|row| row.process.disk_written_bytes.unwrap_or(0) + row.process.deleted_open_bytes)
            .sum();
        processes + self.temp_bytes
    }
}

fn named(folders: &[(PathBuf, u64)]) -> String {
    folders
        .iter()
        .map(|(path, bytes)| {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            format!("{name} ({})", process_resources::size(*bytes))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ask: what the disk has left and what this agent holds, so it can free
/// its own rebuildable output and stop leftovers it does not know about.
pub(crate) fn ask_text(holding: &Holding, free: u64, floor: u64) -> String {
    let size = process_resources::size;
    // Just under the floor both read the same in gigabytes: say it in megabytes then.
    let (free_text, floor_text) = if size(free) == size(floor) {
        (
            format!("{} MB", free / (1024 * 1024)),
            format!("{} MB", floor / (1024 * 1024)),
        )
    } else {
        (size(free), size(floor))
    };
    let mut text = format!(
        "The disk is running out of room: {free_text} free, below the {floor_text} the machine keeps free. You are one of the biggest users. Free what you can rebuild yourself (build output, caches, copies you downloaded), stop anything you started that is not needed any more, and tell me what you freed. Never delete results or source.",
    );
    let processes: Vec<_> = holding
        .processes
        .iter()
        .filter(|row| {
            row.process.disk_written_bytes.unwrap_or(0) > 0 || row.process.deleted_open_bytes > 0
        })
        .collect();
    if !processes.is_empty() {
        text.push_str("\n\nYour processes that write:");
        for row in processes {
            let process = &row.process;
            text.push_str(&format!(
                "\n- pid {} `{}`: wrote {}",
                process.pid,
                process.command,
                size(process.disk_written_bytes.unwrap_or(0)),
            ));
            if process.deleted_open_bytes > 0 {
                text.push_str(&format!(
                    ", keeps {} in deleted files open (freed when it ends)",
                    size(process.deleted_open_bytes)
                ));
            }
            if process.outlived {
                text.push_str(", still running after the command that started it ended");
            }
        }
    }
    if let Some(temp) = &holding.temp {
        text.push_str(&format!(
            "\n\nYour temp folder $TMPDIR ({}): {}",
            temp.display(),
            size(holding.temp_bytes)
        ));
        if !holding.temp_biggest.is_empty() {
            text.push_str(&format!("; biggest: {}", named(&holding.temp_biggest)));
        }
    }
    if let Some(workspace) = &holding.workspace {
        if !holding.workspace_biggest.is_empty() {
            text.push_str(&format!(
                "\n\nBiggest folders in your workspace {}: {}",
                workspace.display(),
                named(&holding.workspace_biggest)
            ));
        }
    }
    text
}

impl KernelRuntimeState {
    fn disk_volume_floor(&self) -> fn(&std::path::Path) -> Option<(u64, u64, u64)> {
        #[cfg(test)]
        if let Some(probe) = self.disk_volume_probe {
            return probe;
        }
        process_resources::volume_floor
    }

    /// One pass at a time, paced from completion rather than its start. A slow
    /// scan or an unanswered launch interaction never accumulates more work.
    pub(super) async fn run_disk_maintenance(&self) {
        let mut pace = Pace::default();
        loop {
            pace.asked.retain(|_, at| at.elapsed() < ASK_AGAIN_AFTER);
            self.tend_disk(&mut pace).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    }

    async fn tend_disk(&self, pace: &mut Pace) {
        let trim = due(&mut pace.trimmed, TRIM_EVERY);
        let state = self.clone();
        let asked = pace.asked.clone();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_scan = ScanCancellation(cancelled.clone());
        // All filesystem work shares one awaited blocking job. In particular,
        // periodic trims cannot overlap the pressure scan or another pass.
        let pressured = tokio::task::spawn_blocking(move || {
            if cancelled.load(Ordering::Acquire) {
                return Vec::new();
            }
            if trim {
                Self::trim_temp(agent_temp_idle());
            }
            let volume_floor = state.disk_volume_floor();
            let pressured = state.pressured_agents(
                |path| {
                    (!cancelled.load(Ordering::Acquire))
                        .then(|| volume_floor(path))
                        .flatten()
                },
                &asked,
            );
            if cancelled.load(Ordering::Acquire)
                || pressured.is_empty()
                || Self::trim_temp(PRESSURE_IDLE).bytes > 0
            {
                Vec::new()
            } else {
                pressured
            }
        })
        .await;
        match pressured {
            Ok(pressured) => self.ask_pressured_agents(pressured, pace).await,
            Err(error) => {
                tracing::warn!(%error, "disk maintenance scan failed; retrying next pass")
            }
        }
    }

    async fn ask_pressured_agents(&self, pressured: Vec<(Holding, u64, u64)>, pace: &mut Pace) {
        for (holding, free, floor) in pressured {
            let key = (holding.session_id.clone(), holding.agent_id.clone());
            if pace
                .asked
                .get(&key)
                .is_some_and(|at| at.elapsed() < ASK_AGAIN_AFTER)
            {
                continue;
            }
            pace.asked.insert(key, Instant::now());
            let text = ask_text(&holding, free, floor);
            match tokio::time::timeout(
                ASK_TIMEOUT,
                self.ask_agent(&holding.session_id, &holding.agent_id, text),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, agent = %holding.agent_id, "the disk-pressure ask was not delivered")
                }
                Err(_) => {
                    tracing::warn!(agent = %holding.agent_id, "disk-pressure prompt admission timed out; retrying after the cooldown")
                }
            }
        }
    }

    fn trim_temp(idle: Duration) -> crate::agent_temp::Trimmed {
        let (Some(root), Some(held)) = (crate::agent_temp::root(), crate::agent_temp::held_paths())
        else {
            return Default::default();
        };
        crate::agent_temp::trim(&root, std::time::SystemTime::now(), idle, &held)
    }

    /// The biggest holders on a volume below its floor that have not been
    /// asked lately, biggest first, with the volume's free bytes and floor.
    fn pressured_agents(
        &self,
        volume_floor: impl Fn(&std::path::Path) -> Option<(u64, u64, u64)>,
        asked: &BTreeMap<(String, String), Instant>,
    ) -> Vec<(Holding, u64, u64)> {
        let mut found = Vec::new();
        let mut volumes = BTreeMap::<u64, (u64, u64)>::new();
        let mut sessions = BTreeMap::<String, Vec<SessionProcess>>::new();
        // Processes and scratch belong to the agent even after a run ends.
        // Retained launches must not multiply that agent's holding.
        for agent in self.owned.agent_store.list_agents() {
            if agent.archived() || agent.remote_execution().is_some() {
                continue;
            }
            let Ok(session) = self.owned.session_store.get_session(agent.session_id()) else {
                continue;
            };
            let temp = crate::agent_temp::dir(session.id(), agent.id());
            let workspace = Some(PathBuf::from(
                agent.worktree_id().unwrap_or(session.worktree_id()),
            ));
            let pressure = [workspace.as_deref(), temp.as_deref()]
                .into_iter()
                .flatten()
                .filter_map(&volume_floor)
                .find(|(_, free, floor)| free < floor);
            let Some((volume, free, floor)) = pressure else {
                continue;
            };
            volumes.insert(volume, (free, floor));
            let processes = sessions.entry(session.id().to_string()).or_insert_with(|| {
                process_resources::for_session(
                    session.id(),
                    &self.owned.provider_store,
                    &self.owned.room_terminals,
                )
            });
            let processes = processes
                .iter()
                .filter(
                    |row| matches!(&row.owner, SessionProcessOwner::Agent(id) if id == agent.id()),
                )
                .cloned()
                .collect();
            found.push((
                volume,
                session.id().to_string(),
                agent.id().to_string(),
                processes,
                temp,
                workspace,
            ));
        }
        found.retain(|(_, session, agent, ..)| {
            asked
                .get(&(session.clone(), agent.clone()))
                .is_none_or(|at| at.elapsed() >= ASK_AGAIN_AFTER)
        });
        // Sizes first, for everyone on a full volume; the folder listings only for those asked.
        let mut holdings: Vec<_> = found
            .into_iter()
            .map(
                |(volume, session_id, agent_id, processes, temp, workspace)| {
                    let mut budget = crate::disk_scan::BUDGET;
                    let temp_bytes = temp
                        .as_deref()
                        .map_or(0, |dir| crate::disk_scan::size_of(dir, &mut budget));
                    (
                        volume,
                        Holding {
                            session_id,
                            agent_id,
                            processes,
                            temp,
                            temp_bytes,
                            temp_biggest: Vec::new(),
                            workspace,
                            workspace_biggest: Vec::new(),
                        },
                    )
                },
            )
            .filter(|(_, holding)| holding.used() >= WORTH_ASKING)
            .collect();
        holdings.sort_by_key(|(_, holding)| std::cmp::Reverse(holding.used()));
        holdings.truncate(ASKED_AT_ONCE);
        holdings
            .into_iter()
            .map(|(volume, mut holding)| {
                let biggest = |dir: &Option<PathBuf>| {
                    dir.as_deref()
                        .map(|dir| crate::disk_scan::biggest_children(dir, FOLDERS_NAMED))
                        .unwrap_or_default()
                };
                holding.temp_biggest = biggest(&holding.temp);
                holding.workspace_biggest = biggest(&holding.workspace);
                let (free, floor) = volumes[&volume];
                (holding, free, floor)
            })
            .collect()
    }

    /// Puts `text` to the agent as a turn of its own: it starts now when the
    /// agent is idle and waits for the end of the turn it is in otherwise.
    async fn ask_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        text: String,
    ) -> Result<(), DaemonError> {
        if self.owned.agent_store.get_agent(agent_id)?.archived() {
            return Ok(());
        }
        let source_attachment_id = self.owned.ensure_agent_trigger_attachment(session_id)?;
        let prompt_id = self.owned.session_store.reserve_prompt_id();
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id,
            &source_attachment_id,
            agent_id,
            &text,
            crate::session::PromptStatus::Queued,
        )
        .with_prompt_origin(crate::session::PromptOrigin::Notification);
        let mut pending = PendingDiskPrompt {
            state: self,
            session: session_id,
            prompt: Some(prompt.id().to_string()),
        };
        let mut submission = self
            .submit_prepared_agent_trigger_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                None,
            )
            .await?;
        pending.prompt = None;
        if let (
            crate::session::PromptSubmissionOutcome::Started { prompt },
            Some(runtime_dispatch),
        ) = (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &runtime_dispatch.session_id,
                &runtime_dispatch.agent_id,
                prompt.id(),
                &runtime_dispatch.provider_run_id,
                "disk-pressure",
            );
        }
        if let Some(runtime_dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(runtime_dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(remote_dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(remote_dispatch);
        }
        Ok(())
    }
}

fn agent_temp_idle() -> Duration {
    crate::agent_temp::idle_window()
}

#[cfg(test)]
mod tests;
