//! Independent diagnostics keep working when runtime workers or shared locks
//! stop advancing. Registrations contain source locations, never request data.
use std::collections::BTreeMap;
use std::panic::Location;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const STALL_THRESHOLD: Duration = Duration::from_secs(2);

struct Operation {
    component: &'static str,
    operation: &'static str,
    location: &'static Location<'static>,
    thread: String,
    started: Instant,
    threshold: Duration,
    reported: bool,
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    operations: BTreeMap<u64, Operation>,
}

pub(crate) struct Watch {
    registry: Arc<Mutex<Registry>>,
    id: u64,
}

#[track_caller]
pub(crate) fn watch(component: &'static str, operation: &'static str) -> Watch {
    watch_at(component, operation, Location::caller())
}

pub(crate) fn watch_at(
    component: &'static str,
    operation: &'static str,
    location: &'static Location<'static>,
) -> Watch {
    register(component, operation, location, STALL_THRESHOLD)
}

#[track_caller]
pub(crate) fn progress_watch(
    component: &'static str,
    operation: &'static str,
    threshold: Duration,
) -> Watch {
    register(component, operation, Location::caller(), threshold)
}

fn register(
    component: &'static str,
    operation: &'static str,
    location: &'static Location<'static>,
    threshold: Duration,
) -> Watch {
    static REGISTRY: OnceLock<Arc<Mutex<Registry>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let worker = Arc::clone(&registry);
        std::thread::Builder::new()
            .name("chariox-stalls".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(250));
                let notices = overdue(
                    &mut worker.lock().unwrap_or_else(|e| e.into_inner()),
                    Instant::now(),
                );
                for mut notice in notices {
                    notice["threads"] = serde_json::json!(thread_names());
                    crate::logging::warn_with_fields(
                        "kernel.watchdog",
                        "kernel operation stalled",
                        notice,
                    );
                }
            })
            .expect("stall watchdog thread must start");
        registry
    });
    let mut state = registry.lock().unwrap_or_else(|e| e.into_inner());
    state.next_id += 1;
    let id = state.next_id;
    state.operations.insert(
        id,
        Operation {
            component,
            operation,
            location,
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string(),
            started: Instant::now(),
            threshold,
            reported: false,
        },
    );
    Watch {
        registry: Arc::clone(registry),
        id,
    }
}

fn overdue(registry: &mut Registry, now: Instant) -> Vec<serde_json::Value> {
    let mut notices = Vec::new();
    for operation in registry.operations.values_mut() {
        let duration = now.saturating_duration_since(operation.started);
        if operation.reported || duration < operation.threshold {
            continue;
        }
        operation.reported = true;
        notices.push(serde_json::json!({"component": operation.component, "operation": operation.operation, "call_site": operation.location.to_string(), "thread": operation.thread, "duration_ms": duration.as_millis()}));
    }
    if !notices.is_empty() {
        let held = registry.operations.values().filter(|operation| operation.operation == "lock_hold").map(|operation| serde_json::json!({"name": operation.component, "call_site": operation.location.to_string(), "thread": operation.thread, "duration_ms": now.saturating_duration_since(operation.started).as_millis()})).collect::<Vec<_>>();
        for notice in &mut notices {
            notice["held_locks"] = serde_json::json!(&held);
        }
    }
    notices
}

fn thread_names() -> String {
    #[cfg(target_os = "linux")]
    {
        let mut names = std::fs::read_dir("/proc/self/task")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                std::fs::read_to_string(entry.path().join("comm"))
                    .ok()
                    .map(|name| format!("{}:{}", entry.file_name().to_string_lossy(), name.trim()))
            })
            .collect::<Vec<_>>();
        names.sort();
        names.join(", ")
    }
    #[cfg(not(target_os = "linux"))]
    {
        "thread names are included with each registered operation".to_string()
    }
}

impl Watch {
    pub(crate) fn progress(&self) {
        if let Some(operation) = self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .operations
            .get_mut(&self.id)
        {
            operation.started = Instant::now();
            operation.reported = false;
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .operations
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_still_blocked_operation_once_with_its_call_site() {
        let now = Instant::now();
        let location = Location::caller();
        let mut registry = Registry::default();
        registry.operations.insert(
            1,
            Operation {
                component: "catalog",
                operation: "lock_hold",
                location,
                thread: "holder".into(),
                started: now,
                threshold: STALL_THRESHOLD,
                reported: false,
            },
        );
        assert!(overdue(&mut registry, now + STALL_THRESHOLD / 2).is_empty());
        let notices = overdue(&mut registry, now + STALL_THRESHOLD);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["call_site"], location.to_string());
        assert_eq!(notices[0]["held_locks"][0]["thread"], "holder");
        assert_eq!(notices[0]["duration_ms"], 2000);
        assert!(overdue(&mut registry, now + STALL_THRESHOLD * 2).is_empty());
    }
}
