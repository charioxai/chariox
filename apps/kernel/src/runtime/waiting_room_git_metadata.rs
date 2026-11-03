//! MP-08 / MP-10 / MP-11: last-known Git labels with bounded background refresh.
use crate::local::WaitingRoomLaunchTarget;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

#[derive(Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Query {
    Launch(String),
    Workspace(String),
    Worktree { worktree: String, workspace: String },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Value {
    Launch(Option<WaitingRoomLaunchTarget>),
    Label(Option<String>),
}
struct Entry {
    value: Option<Value>,
    expires: Instant,
    refreshing: bool,
}
#[derive(Default)]
struct Cache {
    entries: Mutex<HashMap<Query, Entry>>,
    revision: AtomicU64,
    changed: tokio::sync::Notify,
}
static CACHE: OnceLock<Arc<Cache>> = OnceLock::new();
const TTL: Duration = Duration::from_secs(30);
fn cache() -> &'static Arc<Cache> {
    CACHE.get_or_init(Default::default)
}

impl Cache {
    fn begin(&self, query: &Query) -> (Option<Value>, bool) {
        let Ok(mut entries) = self.entries.lock() else {
            return (None, false);
        };
        let previous = entries.get(query).and_then(|e| e.value.clone());
        if entries
            .get(query)
            .is_some_and(|e| e.refreshing || e.expires > Instant::now())
        {
            return (previous, false);
        }
        if entries.len() >= 128 && !entries.contains_key(query) {
            let victim = entries
                .iter()
                .find(|(_, e)| !e.refreshing)
                .map(|(k, _)| k.clone());
            if let Some(victim) = victim {
                entries.remove(&victim);
            } else {
                return (previous, false);
            }
        }
        // Retain a bounded pending entry even when both worker slots are busy.
        entries.entry(query.clone()).or_insert_with(|| Entry {
            value: None,
            expires: Instant::now(),
            refreshing: false,
        });
        // At most two refreshes; repeated snapshots never queue unbounded I/O.
        if entries.values().filter(|e| e.refreshing).count() >= 2 {
            return (previous, false);
        }
        entries.get_mut(query).unwrap().refreshing = true;
        (previous, true)
    }
    fn finish(&self, query: Query, fresh: Option<Value>) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let Some(entry) = entries.get_mut(&query) else {
            return;
        };
        let mut changed = false;
        if let Some(value) = fresh {
            if entry.value.as_ref() != Some(&value) {
                self.revision.fetch_add(1, Ordering::Release);
                changed = true;
            }
            entry.value = Some(value);
        }
        entry.expires = Instant::now() + TTL;
        entry.refreshing = false;
        drop(entries);
        if changed {
            self.changed.notify_waiters();
        }
    }
    fn get(self: &Arc<Self>, query: Query) -> Option<Value> {
        let (previous, refresh) = self.begin(&query);
        if refresh {
            let owned = Arc::clone(self);
            let pending = query.clone();
            if std::thread::Builder::new()
                .name("workspace-metadata".into())
                .spawn(move || {
                    let fresh = refresh_query(pending.clone());
                    owned.finish(pending, fresh);
                })
                .is_err()
            {
                self.finish(query, None);
            }
        }
        previous
    }
    fn refresh_expired(self: &Arc<Self>) {
        // MP-08 / MP-10: projection fast paths also call revision(). Schedule
        // retries there so a failed/capacity-skipped cold read cannot strand
        // its labels behind an unchanged successful-value generation.
        let pending = self
            .entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(_, e)| !e.refreshing && e.expires <= Instant::now())
                    .map(|(query, _)| query.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for query in pending {
            self.get(query);
        }
    }
    fn prime(&self, query: Query) -> Option<Value> {
        let (previous, refresh) = self.begin(&query);
        if !refresh {
            return previous;
        }
        let fresh = refresh_query(query.clone());
        self.finish(query, fresh.clone());
        fresh.or(previous)
    }
}
fn refresh_query(query: Query) -> Option<Value> {
    // Disposable metadata only, outside source workspaces; no config or values.
    let metadata = crate::project_environment::inspect_workspace_metadata(
        &std::env::temp_dir(),
        super::workspace_inspection::Inspection::Labels(query),
        Instant::now() + Duration::from_secs(3),
    )
    .ok()?;
    match metadata {
        super::workspace_inspection::Metadata::Labels(value) => Some(value),
        _ => None,
    }
}
pub(crate) fn revision() -> u64 {
    cache().refresh_expired();
    cache().revision.load(Ordering::Acquire)
}
// MP-08 / MP-10: inventory streams share the same metadata completion clock
// as snapshot readers. Register before checking to avoid a lost completion.
pub(crate) async fn wait_for_change_after(revision: u64) {
    let cache = cache();
    loop {
        let changed = cache.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if cache.revision.load(Ordering::Acquire) != revision {
            return;
        }
        changed.await;
    }
}
pub(crate) fn prime_launch_target() {
    if let Ok(cwd) = std::env::current_dir() {
        cache().prime(Query::Launch(cwd.to_string_lossy().into_owned()));
    }
}
pub(crate) fn launch_target_for(cwd: &std::path::Path) -> Option<WaitingRoomLaunchTarget> {
    match cache().get(Query::Launch(cwd.to_string_lossy().into_owned())) {
        Some(Value::Launch(value)) => value,
        _ => None,
    }
}
pub(crate) fn workspace_label(workspace: &str) -> Option<String> {
    match cache().get(Query::Workspace(workspace.into())) {
        Some(Value::Label(value)) => value,
        _ => None,
    }
}
pub(crate) fn worktree_label(worktree: &str, workspace: &str) -> Option<String> {
    match cache().get(Query::Worktree {
        worktree: worktree.into(),
        workspace: workspace.into(),
    }) {
        Some(Value::Label(value)) => value,
        _ => None,
    }
}

#[path = "waiting_room_git_measurement.rs"]
mod measurement;
pub(crate) use measurement::measure;
#[cfg(test)]
#[path = "waiting_room_git_metadata_tests.rs"]
mod tests;
