//! NC-14: the watcher contract a script speaks, through a real process.
use super::contract::{parse_line, validate, Input, Report};
use super::run::{run, RunEnd, RunSpec};
use crate::local::{
    WatcherCoalesce, WatcherEvent, WatcherEventDeclaration, WatcherManifest, WatcherMode,
    WatcherScript,
};
use std::time::Duration;

fn manifest(command: &[&str]) -> WatcherManifest {
    WatcherManifest {
        name: "Inbox".into(),
        sentence: "Tells you when a mail from the bank arrives.".into(),
        mode: WatcherMode::Poll { every_s: 60 },
        command: command.iter().map(|arg| (*arg).to_owned()).collect(),
        events: vec![
            WatcherEventDeclaration {
                name: "mail".into(),
                keyed: true,
            },
            WatcherEventDeclaration {
                name: "quiet".into(),
                keyed: false,
            },
        ],
        coalesce: WatcherCoalesce::Every,
        debounce_s: 0,
    }
}
fn script(text: &str) -> WatcherScript {
    WatcherScript {
        file_name: "watch.sh".into(),
        text: text.into(),
    }
}

#[test]
fn a_manifest_is_checked_once_at_the_boundary() {
    let ok = manifest(&["sh", "watch.sh"]);
    assert_eq!(validate(&ok, &script("true")), Ok(()));
    let mut fast = ok.clone();
    fast.mode = WatcherMode::Poll { every_s: 5 };
    assert!(validate(&fast, &script("true")).is_err());
    let mut twice = ok.clone();
    twice.events.push(twice.events[0].clone());
    assert!(validate(&twice, &script("true")).is_err());
    let mut silent = ok.clone();
    silent.events.clear();
    assert!(validate(&silent, &script("true")).is_err());
    assert!(validate(
        &ok,
        &WatcherScript {
            file_name: "../escape.sh".into(),
            text: "true".into(),
        }
    )
    .is_err());
}

#[test]
fn each_output_line_is_one_known_report_and_anything_else_fails_the_run() {
    let m = manifest(&["sh", "watch.sh"]);
    assert_eq!(
        parse_line(
            &m,
            r#"{"event":"mail","key":"m1","text":"Bank: statement ready"}"#
        ),
        Ok(Report::Event(WatcherEvent {
            event: "mail".into(),
            key: Some("m1".into()),
            text: "Bank: statement ready".into(),
        }))
    );
    assert_eq!(
        parse_line(&m, r#"{"reading":"2 unread"}"#),
        Ok(Report::Reading("2 unread".into()))
    );
    assert_eq!(
        parse_line(&m, r#"{"history":false}"#),
        Ok(Report::History(false))
    );
    assert_eq!(
        parse_line(&m, r#"{"cursor":{"uid":42}}"#),
        Ok(Report::Cursor(serde_json::json!({"uid":42})))
    );
    for refused in [
        "not json",
        r#"{"surprise":1}"#,
        r#"{"event":"spam","key":"k","text":"undeclared"}"#,
        r#"{"event":"mail","text":"keyed event without a key"}"#,
        r#"{"event":"quiet","key":"k","text":"unkeyed event with a key"}"#,
        r#"{"event":"mail","key":"k","text":"x","payload":{}}"#,
        r#"{"candidate":{"text":"needs a judge"}}"#,
    ] {
        assert!(parse_line(&m, refused).is_err(), "{refused}");
    }
}

async fn run_script(
    text: &str,
    limit: Duration,
) -> (RunEnd, Vec<Report>, crate::test_support::TestTempDir) {
    let dir = crate::test_support::TestTempDir::new("watcher-run");
    std::fs::write(dir.join("watch.sh"), text).unwrap();
    let (_stop, stop) = tokio::sync::watch::channel(false);
    let mut reports = Vec::new();
    let end = run(
        RunSpec {
            dir: dir.to_path_buf(),
            manifest: manifest(&["sh", "watch.sh"]),
            input: Input {
                cursor: Some(serde_json::json!({"uid": 41})),
                now_ms: 1_000,
                dry_run: false,
                since_ms: None,
            },
            claim: format!("watcher:test-{}", std::process::id()),
            limit: Some(limit),
        },
        stop,
        |report| {
            reports.push(report);
            Ok(())
        },
    )
    .await;
    (end, reports, dir)
}

#[tokio::test]
async fn a_run_reads_its_cursor_on_stdin_and_reports_its_lines_in_order() {
    let (end, reports, _dir) = run_script(
        r#"read input
uid=$(printf '%s' "$input" | sed 's/.*"uid":\([0-9]*\).*/\1/')
echo '{"reading":"1 new mail"}'
echo "{\"event\":\"mail\",\"key\":\"m$((uid+1))\",\"text\":\"Bank: statement ready\"}"
echo "{\"cursor\":{\"uid\":$((uid+1))}}"
"#,
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert_eq!(
        reports,
        vec![
            Report::Reading("1 new mail".into()),
            Report::Event(WatcherEvent {
                event: "mail".into(),
                key: Some("m42".into()),
                text: "Bank: statement ready".into(),
            }),
            Report::Cursor(serde_json::json!({"uid": 42})),
        ]
    );
}

#[tokio::test]
async fn a_failing_script_says_why_with_the_end_of_its_error_output() {
    let (end, _, _dir) = run_script(
        "echo 'curl: (6) Could not resolve host' >&2\nexit 3\n",
        Duration::from_secs(30),
    )
    .await;
    let RunEnd::Failed { reason, output } = end else {
        panic!("expected a failure, got {end:?}");
    };
    assert!(reason.contains('3'), "{reason}");
    assert!(output.contains("Could not resolve host"), "{output}");
    let (end, _, _dir) = run_script(
        "echo '{\"event\":\"spam\",\"text\":\"x\"}'\n",
        Duration::from_secs(30),
    )
    .await;
    assert!(
        matches!(&end, RunEnd::Failed { reason, .. } if reason.contains("not declared")),
        "{end:?}"
    );
}

#[tokio::test]
async fn a_run_over_its_time_is_ended_with_its_whole_process_tree() {
    let (end, _, dir) = run_script(
        "sleep 30 >/dev/null 2>&1 &\necho $! > child.pid\nsleep 30\n",
        Duration::from_secs(1),
    )
    .await;
    assert!(
        matches!(&end, RunEnd::Failed { reason, .. } if reason.contains("longer than")),
        "{end:?}"
    );
    let child = std::fs::read_to_string(dir.join("child.pid")).unwrap();
    let stat = std::path::Path::new("/proc")
        .join(child.trim())
        .join("stat");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    // Gone, or a zombie its new parent has not reaped yet.
    while std::fs::read_to_string(&stat).is_ok_and(|stat| !stat.contains(") Z ")) {
        assert!(
            std::time::Instant::now() < deadline,
            "the background child survived its run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
