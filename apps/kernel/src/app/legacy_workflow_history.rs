use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::session::WorkflowRun;

type RunOrderKey = (u64, String);

#[derive(Debug, Clone, Default)]
pub(crate) struct LegacyWorkflowHistoryStore {
    inner: Arc<RwLock<LegacyWorkflowHistoryIndex>>,
}

#[derive(Debug, Default)]
struct LegacyWorkflowHistoryIndex {
    by_session: BTreeMap<String, BTreeMap<RunOrderKey, Arc<WorkflowRun>>>,
    by_session_run_id: BTreeMap<String, BTreeMap<String, Arc<WorkflowRun>>>,
    by_workflow: BTreeMap<(String, String), BTreeMap<RunOrderKey, Arc<WorkflowRun>>>,
}

#[derive(Debug)]
pub(crate) struct LegacyWorkflowRunPage {
    pub(crate) workflow_runs: Vec<WorkflowRun>,
    pub(crate) has_more: bool,
}

impl LegacyWorkflowHistoryStore {
    pub(crate) fn insert_all(&self, workflow_runs: Vec<(String, WorkflowRun)>) {
        let mut index = self
            .inner
            .write()
            .expect("legacy workflow history lock poisoned");
        for (session_id, workflow_run) in workflow_runs {
            let key = (workflow_run.created_at_ms(), workflow_run.id().to_string());
            let workflow_id = workflow_run.workflow_id().to_string();
            let workflow_run = Arc::new(workflow_run);
            index
                .by_session
                .entry(session_id.clone())
                .or_default()
                .insert(key.clone(), Arc::clone(&workflow_run));
            index
                .by_session_run_id
                .entry(session_id.clone())
                .or_default()
                .insert(workflow_run.id().to_string(), Arc::clone(&workflow_run));
            index
                .by_workflow
                .entry((session_id, workflow_id))
                .or_default()
                .insert(key, workflow_run);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner
            .read()
            .expect("legacy workflow history lock poisoned")
            .by_session
            .values()
            .all(BTreeMap::is_empty)
    }

    pub(crate) fn len(&self) -> usize {
        self.inner
            .read()
            .expect("legacy workflow history lock poisoned")
            .by_session
            .values()
            .map(BTreeMap::len)
            .sum()
    }

    pub(crate) fn next_chunk(&self, limit: usize) -> Vec<(String, WorkflowRun)> {
        let index = self
            .inner
            .read()
            .expect("legacy workflow history lock poisoned");
        index
            .by_session
            .iter()
            .flat_map(|(session_id, runs)| {
                runs.values()
                    .map(move |run| (session_id.clone(), run.as_ref().clone()))
            })
            .take(limit)
            .collect()
    }

    pub(crate) fn remove_committed(&self, workflow_runs: &[(String, WorkflowRun)]) {
        let mut index = self
            .inner
            .write()
            .expect("legacy workflow history lock poisoned");
        for (session_id, workflow_run) in workflow_runs {
            let key = (workflow_run.created_at_ms(), workflow_run.id().to_string());
            let workflow_key = (session_id.clone(), workflow_run.workflow_id().to_string());
            if let Some(runs) = index.by_session.get_mut(session_id) {
                runs.remove(&key);
                if runs.is_empty() {
                    index.by_session.remove(session_id);
                }
            }
            if let Some(runs) = index.by_session_run_id.get_mut(session_id) {
                runs.remove(workflow_run.id());
                if runs.is_empty() {
                    index.by_session_run_id.remove(session_id);
                }
            }
            if let Some(runs) = index.by_workflow.get_mut(&workflow_key) {
                runs.remove(&key);
                if runs.is_empty() {
                    index.by_workflow.remove(&workflow_key);
                }
            }
        }
    }

    pub(crate) fn list_page(
        &self,
        session_id: &str,
        workflow_id: Option<&str>,
        before: Option<(u64, &str)>,
        limit: usize,
    ) -> LegacyWorkflowRunPage {
        let index = self
            .inner
            .read()
            .expect("legacy workflow history lock poisoned");
        let runs = match workflow_id {
            Some(workflow_id) => index
                .by_workflow
                .get(&(session_id.to_string(), workflow_id.to_string())),
            None => index.by_session.get(session_id),
        };
        let Some(runs) = runs else {
            return LegacyWorkflowRunPage {
                workflow_runs: Vec::new(),
                has_more: false,
            };
        };
        let upper_bound = before
            .map(|(created_at_ms, run_id)| (created_at_ms, run_id.to_string()))
            .unwrap_or_else(|| (u64::MAX, String::from("\u{10ffff}")));
        let mut selected = runs
            .range(..upper_bound)
            .rev()
            .take(limit.saturating_add(1))
            .map(|(_, run)| run.as_ref().clone())
            .collect::<Vec<_>>();
        let has_more = selected.len() > limit;
        selected.truncate(limit);
        LegacyWorkflowRunPage {
            workflow_runs: selected,
            has_more,
        }
    }

    pub(crate) fn resolve(&self, session_id: &str, workflow_run_ref: &str) -> Option<WorkflowRun> {
        let normalized_ref = workflow_run_ref.trim().to_lowercase();
        let index = self
            .inner
            .read()
            .expect("legacy workflow history lock poisoned");
        let runs = index.by_session_run_id.get(session_id)?;
        if let Some(run) = runs.get(&normalized_ref) {
            return Some(run.as_ref().clone());
        }
        let mut matches = runs.range(normalized_ref.clone()..).map(|(_, run)| run);
        let first = matches
            .next()
            .filter(|run| run.id().starts_with(&normalized_ref))?
            .as_ref()
            .clone();
        if matches
            .next()
            .is_some_and(|run| run.id().starts_with(&normalized_ref))
        {
            return None;
        }
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{WorkflowRun, WorkflowRunStatus};

    fn completed_run(id: &str) -> WorkflowRun {
        let mut run = WorkflowRun::new(
            id,
            "workflow-1",
            "endpoint-1",
            "node-1",
            None,
            None,
            Vec::new(),
            Vec::new(),
        );
        run.set_status(WorkflowRunStatus::Completed);
        run
    }

    #[test]
    fn failed_chunks_remain_queryable_and_committed_chunks_retire() {
        let store = LegacyWorkflowHistoryStore::default();
        let first = completed_run("run-1");
        let second = completed_run("run-2");
        store.insert_all(vec![
            ("session-1".to_string(), first.clone()),
            ("session-1".to_string(), second.clone()),
        ]);

        let page = store.list_page("session-1", Some("workflow-1"), None, 1);
        assert_eq!(page.workflow_runs[0].id(), "run-2");
        assert!(page.has_more);
        assert_eq!(store.resolve("session-1", "run-1"), Some(first.clone()));

        store.remove_committed(&[("session-1".to_string(), first)]);
        assert!(store.resolve("session-1", "run-1").is_none());
        assert_eq!(store.resolve("session-1", "run-2"), Some(second));
    }
}
