//! NC-14 / NC-15 / NC-21: an agent makes a watcher with its tools; the kernel runs it on the
//! transport tick and delivers what it fires to the agents that follow it, through the one
//! notification path.
use super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt_and_config;
use super::*;
use crate::config::DaemonConfig;
use crate::local::{WatcherHealth, WatcherRequest, WatcherState};
use serde_json::json;
use std::time::Duration;

const SCRIPT: &str = r#"read input
case "$input" in
  *'"dry_run":true'*)
    echo '{"history":false}'
    echo '{"event":"mail","key":"m1","text":"Bank: statement ready"}'
    exit 0;;
  *'"seen":1'*)
    echo '{"reading":"no new mail"}'
    exit 0;;
esac
echo '{"reading":"1 new mail"}'
echo '{"event":"mail","key":"m1","text":"Bank: statement ready"}'
echo '{"cursor":{"seen":1}}'
"#;

struct Rig {
    _root: crate::test_support::TestWorktree,
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
}

async fn rig() -> Rig {
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (root, runtime, session_id, agent_id, _, _, _) =
        runtime_with_active_prompt_and_config(config, &[]).await;
    Rig {
        _root: root,
        runtime,
        session_id,
        agent_id,
    }
}

impl Rig {
    async fn tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.runtime
            .dispatch_meta_runtime_tool_call_for_agent(
                &self.session_id,
                &self.agent_id,
                name,
                arguments,
            )
            .await
            .map(|result| result.payload)
            .map_err(|e| e.to_string())
    }

    async fn create(&self, script: &str) -> (String, String) {
        self.create_in_mode(script, json!({"poll": {"every_s": 60}}))
            .await
    }

    async fn create_in_mode(&self, script: &str, mode: serde_json::Value) -> (String, String) {
        let created = self
            .tool(
                "chariox.watcher.create",
                json!({
                    "manifest": {
                        "name": "Bank mail",
                        "sentence": "Tells you when a mail from the bank arrives.",
                        "mode": mode,
                        "command": ["sh", "watch.sh"],
                        "events": [{"name": "mail", "keyed": true}],
                    },
                    "script": {"file_name": "watch.sh", "text": script},
                }),
            )
            .await
            .unwrap();
        (
            created["watcher"]["watcher_id"].as_str().unwrap().into(),
            created["watcher"]["source_id"].as_str().unwrap().into(),
        )
    }

    async fn record_until(
        &self,
        watcher_id: &str,
        done: impl Fn(&crate::durable_state::workflow_notifications::watcher::WatcherRecord) -> bool,
    ) -> crate::durable_state::workflow_notifications::watcher::WatcherRecord {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let record = self
                .runtime
                .owned
                .durable_state_store
                .watcher(watcher_id)
                .unwrap()
                .unwrap();
            if done(&record) {
                return record;
            }
            assert!(std::time::Instant::now() < deadline, "{record:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
            // The transport tick.
            self.runtime.schedule_watchers();
        }
    }

    /// A stream made, dry run and turned on with the tools.
    async fn stream(&self, script: &str) -> String {
        let (watcher_id, _) = self.create_in_mode(script, json!("stream")).await;
        self.tool("chariox.watcher.dry_run", json!({"watcher_id": watcher_id}))
            .await
            .unwrap();
        self.tool(
            "chariox.watcher.activate",
            json!({"watcher_id": watcher_id}),
        )
        .await
        .unwrap();
        watcher_id
    }

    async fn pause(&self, watcher_id: &str, paused: bool) {
        self.runtime
            .execute_watcher_request(
                WatcherRequest::Pause {
                    watcher_id: watcher_id.into(),
                    paused,
                },
                "local",
            )
            .await
            .unwrap();
    }

    async fn deliver(&self) {
        for _ in 0..2 {
            self.runtime.owned.route_workflow_notifications();
            self.runtime.route_agent_notifications().await;
        }
    }

    fn queued_prompts(&self) -> Vec<String> {
        let session = self
            .runtime
            .owned
            .session_snapshot(&self.session_id)
            .unwrap();
        let (_, queued) = self
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &self.agent_id);
        queued.iter().map(|p| p.prompt().to_owned()).collect()
    }
}

#[tokio::test]
async fn an_agent_makes_a_watcher_and_its_followers_get_each_event_once() {
    let rig = rig().await;
    let (watcher_id, source_id) = rig.create(SCRIPT).await;
    let refused = rig
        .tool(
            "chariox.watcher.activate",
            json!({"watcher_id": watcher_id}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("dry run"), "{refused}");
    let dry = rig
        .tool("chariox.watcher.dry_run", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    assert_eq!(dry["history"], false);
    assert_eq!(dry["events"][0]["text"], "Bank: statement ready");
    // The dry run delivered nothing and left the cursor alone.
    let record = rig.record_until(&watcher_id, |_| true).await;
    assert_eq!((record.fired_count, record.cursor), (0, None));

    let sources = rig
        .tool("chariox.notification_sources", json!({}))
        .await
        .unwrap();
    assert!(sources.to_string().contains(&source_id), "{sources}");
    rig.tool(
        "chariox.notification_subscribe",
        json!({"source": {"workflow": {"source_id": source_id}}, "prompt": "Tell me when a statement is ready."}),
    )
    .await
    .unwrap();
    let on = rig
        .tool(
            "chariox.watcher.activate",
            json!({"watcher_id": watcher_id}),
        )
        .await
        .unwrap();
    assert_eq!(on["watcher"]["state"], "active");
    let record = rig
        .record_until(&watcher_id, |record| record.fired_count == 1)
        .await;
    assert_eq!(record.cursor, Some(json!({"seen": 1})));
    assert_eq!(record.reading.as_deref(), Some("1 new mail"));
    rig.deliver().await;
    let delivered = rig
        .queued_prompts()
        .into_iter()
        .filter(|prompt| prompt.contains("Bank: statement ready"))
        .count();
    assert_eq!(delivered, 1);

    // Check now runs it again from its cursor: nothing new fires.
    rig.runtime
        .execute_watcher_request(
            WatcherRequest::CheckNow {
                watcher_id: watcher_id.clone(),
            },
            "local",
        )
        .await
        .unwrap();
    let record = rig
        .record_until(&watcher_id, |record| {
            record.reading.as_deref() == Some("no new mail")
        })
        .await;
    assert_eq!(record.fired_count, 1);

    let status = rig
        .tool("chariox.watcher.status", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    assert_eq!(status["watcher"]["health"]["state"], "ok");
    rig.tool("chariox.watcher.remove", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    let sources = rig
        .tool("chariox.notification_sources", json!({}))
        .await
        .unwrap();
    assert!(!sources.to_string().contains(&source_id), "{sources}");
}

// c#665: the list names every follower of a source, wherever the follower is: a follower on
// another kernel is held by this source kernel and is listed beside the Room's own.
#[tokio::test]
async fn the_watcher_list_names_the_followers_of_its_sources_on_every_kernel() {
    let rig = rig().await;
    let (watcher_id, source_id) = rig.create(SCRIPT).await;
    rig.tool("chariox.watcher.dry_run", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    rig.tool(
        "chariox.watcher.activate",
        json!({"watcher_id": watcher_id}),
    )
    .await
    .unwrap();
    rig.tool(
        "chariox.notification_subscribe",
        json!({"source": {"workflow": {"source_id": source_id}}, "prompt": "Tell me when a statement is ready."}),
    )
    .await
    .unwrap();
    let store = &rig.runtime.owned.durable_state_store;
    let local = store
        .notification_inventory("local")
        .unwrap()
        .1
        .into_iter()
        .find(|follower| follower.source_id == source_id)
        .unwrap();
    // As the source kernel stores the follow another kernel asks of it.
    let mut remote = local.clone();
    remote.subscription_id = "remote-follow".into();
    remote.target_kernel_id = "kernel-b".into();
    remote.session_id = "room-b".into();
    store
        .notify(
            crate::durable_state::workflow_notifications::NotificationOperation::SourceAttach {
                subscription: remote,
                generation: 1,
            },
        )
        .unwrap();
    let listed = rig
        .runtime
        .execute_watcher_request(WatcherRequest::List {}, "local")
        .await
        .unwrap();
    let LocalDaemonResponse::Watchers { followers, .. } = listed else {
        panic!("{listed:?}");
    };
    let mut followers: Vec<_> = followers
        .iter()
        .map(|follower| {
            (
                follower.subscription_id.as_str(),
                follower.target_kernel_id.as_str(),
            )
        })
        .collect();
    followers.sort();
    let mut expected = vec![
        (
            local.subscription_id.as_str(),
            local.target_kernel_id.as_str(),
        ),
        ("remote-follow", "kernel-b"),
    ];
    expected.sort();
    assert_eq!(followers, expected);
}

#[tokio::test]
async fn a_failing_watcher_wakes_the_agent_that_made_it_once() {
    let rig = rig().await;
    let (watcher_id, source_id) = rig
        .create("read input\ncase \"$input\" in *'\"dry_run\":true'*) exit 0;; esac\necho 'login expired' >&2\nexit 4\n")
        .await;
    rig.tool("chariox.watcher.dry_run", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    rig.tool(
        "chariox.watcher.activate",
        json!({"watcher_id": watcher_id}),
    )
    .await
    .unwrap();
    let record = rig
        .record_until(&watcher_id, |record| {
            matches!(record.health, WatcherHealth::Failing { .. })
        })
        .await;
    assert_eq!(record.state, WatcherState::Active);
    let status = rig
        .tool("chariox.watcher.status", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
    assert!(
        status["last_failure_output"]
            .as_str()
            .is_some_and(|output| output.contains("login expired")),
        "{status}"
    );
    let told = rig
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&rig.session_id, &rig.agent_id, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "watcher_health" && event.source_id == source_id)
        .collect::<Vec<_>>();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].payload["health"]["state"], "failing");
    // A second failure is the same failing state: nobody is woken again.
    rig.runtime
        .execute_watcher_request(
            WatcherRequest::CheckNow {
                watcher_id: watcher_id.clone(),
            },
            "local",
        )
        .await
        .unwrap();
    let first = record.last_check_ms;
    rig.record_until(&watcher_id, |record| record.last_check_ms != first)
        .await;
    let again = rig
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&rig.session_id, &rig.agent_id, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "watcher_health")
        .count();
    assert_eq!(again, 1);
    rig.tool("chariox.watcher.remove", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
}

/// A stream's script in `dir`: it answers a dry run at once and otherwise runs `body`.
fn stream_script(dir: &std::path::Path, body: &str) -> String {
    format!(
        "read input\ncase \"$input\" in *'\"dry_run\":true'*) exit 0;; esac\ncd '{}'\n{body}\n",
        dir.display()
    )
}

/// Whether the loop that writes `file` is still counting.
async fn counting(file: &std::path::Path) -> bool {
    let before = std::fs::read_to_string(file).unwrap_or_default();
    tokio::time::sleep(Duration::from_millis(600)).await;
    before != std::fs::read_to_string(file).unwrap_or_default()
}

fn starts(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("starts"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn pausing_a_stream_that_closed_its_output_ends_it() {
    let rig = rig().await;
    let dir = crate::test_support::TestTempDir::new("watcher-closed-stream");
    let watcher_id = rig
        .stream(&stream_script(
            dir.path(),
            "exec 1>&-\ni=0; while :; do i=$((i+1)); echo $i > beat; sleep 0.1; done",
        ))
        .await;
    rig.record_until(&watcher_id, |_| dir.path().join("beat").exists())
        .await;
    rig.pause(&watcher_id, true).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while counting(&dir.path().join("beat")).await {
        assert!(
            std::time::Instant::now() < deadline,
            "the paused stream is still running"
        );
    }
    rig.tool("chariox.watcher.remove", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stream_paused_and_resumed_at_once_keeps_its_new_run() {
    let rig = rig().await;
    let dir = crate::test_support::TestTempDir::new("watcher-resumed-stream");
    let watcher_id = rig
        .stream(&stream_script(
            dir.path(),
            "echo $$ >> starts\ni=0; while :; do i=$((i+1)); echo $i > beat.$$; sleep 0.1; done",
        ))
        .await;
    rig.record_until(&watcher_id, |_| starts(dir.path()).len() == 1)
        .await;
    rig.pause(&watcher_id, true).await;
    rig.pause(&watcher_id, false).await;
    rig.record_until(&watcher_id, |_| starts(dir.path()).len() >= 2)
        .await;
    // The first run's cleanup has had its time: the second run is the one running, unharmed.
    let settled = std::time::Instant::now() + Duration::from_secs(2);
    let record = rig
        .record_until(&watcher_id, |_| std::time::Instant::now() >= settled)
        .await;
    assert_eq!(record.health, WatcherHealth::Ok);
    let started = starts(dir.path());
    assert_eq!(started.len(), 2, "{started:?}");
    assert!(counting(&dir.path().join(format!("beat.{}", started[1]))).await);
    rig.tool("chariox.watcher.remove", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stream_that_holds_too_much_without_a_cursor_fails_and_commits_nothing() {
    let rig = rig().await;
    let dir = crate::test_support::TestTempDir::new("watcher-uncommitted-stream");
    let watcher_id = rig
        .stream(&stream_script(
            dir.path(),
            r#"i=0; while [ $i -lt 20000 ]; do echo "{\"event\":\"mail\",\"key\":\"m$i\",\"text\":\"Bank: statement $i\"}"; i=$((i+1)); done
exec sleep 600"#,
        ))
        .await;
    let record = rig
        .record_until(&watcher_id, |record| {
            matches!(record.health, WatcherHealth::Failing { .. })
        })
        .await;
    let WatcherHealth::Failing { reason, .. } = &record.health else {
        unreachable!()
    };
    assert!(reason.contains("without a cursor line"), "{reason}");
    assert_eq!((record.fired_count, record.cursor.clone()), (0, None));
    rig.tool("chariox.watcher.remove", json!({"watcher_id": watcher_id}))
        .await
        .unwrap();
}
