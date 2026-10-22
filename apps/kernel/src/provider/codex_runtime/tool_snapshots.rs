//! MP-08 / MP-10: last-wins Codex tool snapshots, at most 5 live updates/s
//! per tool. Text is never delayed; final tool/turn state bypasses the window.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::provider::output_ready::ProviderEventNotifier;
use crate::terminal::TerminalOutputKind;

use super::CodexOutputChunk;

const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Default)]
pub(super) struct ToolSnapshots {
    pending: BTreeMap<String, CodexOutputChunk>,
    emitted_at: BTreeMap<String, Instant>,
    wake: Option<(Instant, tokio::task::JoinHandle<()>)>,
}

impl ToolSnapshots {
    pub(super) fn coalesce(
        &mut self,
        chunks: Vec<CodexOutputChunk>,
        force: bool,
        now: Instant,
        notifier: &ProviderEventNotifier,
    ) -> Vec<CodexOutputChunk> {
        // Reverse filtering retains the final occurrence at its original
        // position, including when assistant text separates tool updates.
        let mut seen = BTreeSet::new();
        let mut latest = chunks
            .into_iter()
            .rev()
            .filter(|chunk| snapshot_key(chunk).is_none_or(|key| seen.insert(key.to_string())))
            .collect::<Vec<_>>();
        latest.reverse();
        let mut result = Vec::with_capacity(latest.len());
        for chunk in latest {
            let Some(key) = snapshot_key(&chunk).map(str::to_string) else {
                result.push(chunk);
                continue;
            };
            let terminal = serde_json::from_slice::<serde_json::Value>(&chunk.bytes)
                .ok()
                .is_some_and(|value| {
                    matches!(
                        value["status"].as_str(),
                        Some(
                            "completed"
                                | "failed"
                                | "error"
                                | "declined"
                                | "cancelled"
                                | "canceled"
                                | "interrupted"
                        )
                    )
                });
            if force || terminal || self.is_due(&key, now) {
                self.pending.remove(&key);
                self.emitted_at.insert(key, now);
                result.push(chunk);
            } else {
                self.pending.insert(key, chunk);
            }
        }
        let due = self
            .pending
            .keys()
            .filter(|key| force || self.is_due(key, now))
            .cloned()
            .collect::<Vec<_>>();
        for key in due {
            result.push(self.pending.remove(&key).expect("pending snapshot exists"));
            self.emitted_at.insert(key, now);
        }
        if force {
            self.emitted_at.clear();
        }
        self.schedule_wake(now, notifier);
        result
    }

    fn is_due(&self, key: &str, now: Instant) -> bool {
        self.emitted_at
            .get(key)
            .is_none_or(|at| now.saturating_duration_since(*at) >= SNAPSHOT_INTERVAL)
    }

    fn schedule_wake(&mut self, now: Instant, notifier: &ProviderEventNotifier) {
        if self.pending.is_empty() {
            if let Some((_, wake)) = self.wake.take() {
                wake.abort();
            }
            return;
        }
        let delay = self
            .pending
            .keys()
            .filter_map(|key| self.emitted_at.get(key))
            .map(|at| SNAPSHOT_INTERVAL.saturating_sub(now.saturating_duration_since(*at)))
            .min()
            .unwrap_or_default();
        let deadline = now + delay;
        if self
            .wake
            .as_ref()
            .is_some_and(|(at, wake)| *at > now && *at <= deadline && !wake.is_finished())
        {
            return;
        }
        if let Some((_, wake)) = self.wake.take() {
            wake.abort();
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let notifier = notifier.clone();
            self.wake = Some((
                deadline,
                handle.spawn(async move {
                    tokio::time::sleep(delay).await;
                    notifier.notify();
                }),
            ));
        }
    }
}

fn snapshot_key(chunk: &CodexOutputChunk) -> Option<&str> {
    (chunk.kind == TerminalOutputKind::ProviderTool)
        .then_some(chunk.merge_key.as_deref())
        .flatten()
}

impl Drop for ToolSnapshots {
    fn drop(&mut self) {
        if let Some((_, wake)) = self.wake.take() {
            wake.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool(key: &str, status: &str, output: &str) -> CodexOutputChunk {
        CodexOutputChunk {
            kind: TerminalOutputKind::ProviderTool,
            merge_key: Some(key.into()),
            bytes: serde_json::to_vec(
                &serde_json::json!({"id":key,"status":status,"output":output}),
            )
            .unwrap(),
        }
    }
    fn text() -> CodexOutputChunk {
        CodexOutputChunk {
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("text".into()),
            bytes: b"answer".to_vec(),
        }
    }
    #[test]
    fn panes_stream_snapshots_keep_latest_per_tool_and_original_text_order() {
        let mut snapshots = ToolSnapshots::default();
        let now = Instant::now();
        let latest = tool("a", "running", "latest");
        let rows = snapshots.coalesce(
            vec![
                tool("a", "running", "old"),
                tool("b", "running", "other"),
                text(),
                latest.clone(),
            ],
            false,
            now,
            &Default::default(),
        );
        assert_eq!(rows, vec![tool("b", "running", "other"), text(), latest]);
    }
    #[test]
    fn panes_stream_snapshot_rate_is_bounded_and_final_output_bypasses_window() {
        let mut snapshots = ToolSnapshots::default();
        let now = Instant::now();
        let notifier = ProviderEventNotifier::default();
        assert_eq!(
            snapshots
                .coalesce(vec![tool("a", "running", "0")], false, now, &notifier)
                .len(),
            1
        );
        for n in 1..200 {
            assert!(snapshots
                .coalesce(
                    vec![tool("a", "running", &n.to_string())],
                    false,
                    now + Duration::from_millis(n),
                    &notifier
                )
                .is_empty());
        }
        assert_eq!(
            snapshots.coalesce(vec![], false, now + SNAPSHOT_INTERVAL, &notifier),
            vec![tool("a", "running", "199")]
        );
        assert!(snapshots
            .coalesce(
                vec![tool("a", "running", "pending")],
                false,
                now + SNAPSHOT_INTERVAL,
                &notifier
            )
            .is_empty());
        let final_row = tool("a", "completed", "FULL reconciled prefix and tail");
        assert_eq!(
            snapshots.coalesce(
                vec![final_row.clone()],
                false,
                now + SNAPSHOT_INTERVAL,
                &notifier
            ),
            vec![final_row]
        );
        assert!(snapshots.pending.is_empty());
        assert!(snapshots
            .coalesce(
                vec![tool("a", "running", "turn end")],
                false,
                now + SNAPSHOT_INTERVAL,
                &notifier
            )
            .is_empty());
        assert_eq!(
            snapshots.coalesce(vec![text()], true, now + SNAPSHOT_INTERVAL, &notifier),
            vec![text(), tool("a", "running", "turn end")]
        );
    }
    #[tokio::test(start_paused = true)]
    async fn panes_stream_new_pending_tool_advances_the_narrow_wake() {
        let signal = crate::provider::output_ready::ProviderOutputReadySignal::default();
        let notifier = ProviderEventNotifier::default();
        notifier.bind("run".into(), signal.clone());
        signal.take_ready_provider_run_ids();
        let mut snapshots = ToolSnapshots::default();
        let now = Instant::now();
        snapshots.coalesce(
            vec![tool("b", "running", "first")],
            false,
            now - Duration::from_millis(100),
            &notifier,
        );
        snapshots.coalesce(vec![tool("a", "running", "first")], false, now, &notifier);
        snapshots.coalesce(
            vec![tool("a", "running", "last")],
            false,
            now + Duration::from_millis(1),
            &notifier,
        );
        snapshots.coalesce(
            vec![tool("b", "running", "last")],
            false,
            now + Duration::from_millis(1),
            &notifier,
        );
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            signal.take_ready_provider_run_ids(),
            BTreeSet::from(["run".into()])
        );
    }

    #[tokio::test]
    async fn panes_stream_pending_snapshot_wakes_without_another_provider_event() {
        let signal = crate::provider::output_ready::ProviderOutputReadySignal::default();
        let notifier = ProviderEventNotifier::default();
        notifier.bind("run".into(), signal.clone());
        signal.take_ready_provider_run_ids();
        let mut snapshots = ToolSnapshots::default();
        let now = Instant::now();
        snapshots.coalesce(vec![tool("a", "running", "first")], false, now, &notifier);
        assert!(snapshots
            .coalesce(vec![tool("a", "running", "last")], false, now, &notifier)
            .is_empty());
        tokio::time::timeout(
            Duration::from_millis(500),
            signal.wait_for_change_after(signal.sequence()),
        )
        .await
        .unwrap();
        assert!(now.elapsed() >= SNAPSHOT_INTERVAL);
        assert_eq!(
            snapshots.coalesce(vec![], false, Instant::now(), &notifier),
            vec![tool("a", "running", "last")]
        );
        assert!(snapshots.wake.is_none());
    }
}
