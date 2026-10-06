use super::*;
use serde_json::json;

fn turn(prompt: &str, agent: &str, parent: Option<&str>, amount: u64) -> TurnUsage {
    let usage = super::super::codex_usage(&json!({"inputTokens":amount,"cachedInputTokens":0,"outputTokens":1,"reasoningOutputTokens":0})).unwrap();
    let mut t = TurnUsage {
        session_id: "session".into(),
        agent_id: agent.into(),
        parent_agent_id: parent.map(str::to_string),
        prompt_id: prompt.into(),
        provider_run_id: "run".into(),
        provider: "codex".into(),
        model: "gpt-5.3-codex".into(),
        completed: true,
        usage: Some(usage),
        provider_counters: Some(usage),
        api_equivalent_nanodollars: None,
        price_table_version: 0,
        price_table_date: String::new(),
    };
    stamp(&mut t);
    t
}
#[test]
fn mp08_mp10_mp11_cumulative_reset_missing_and_reasoning_are_checked() {
    let a = super::super::codex_usage(&json!({"inputTokens":100,"cachedInputTokens":40,"outputTokens":10,"reasoningOutputTokens":6})).unwrap();
    let b = super::super::codex_usage(&json!({"inputTokens":130,"cachedInputTokens":60,"outputTokens":15,"reasoningOutputTokens":8})).unwrap();
    let diff = b.difference(a).unwrap();
    assert_eq!(diff.input, Some(30));
    assert_eq!(diff.cached_input, Some(20));
    assert_eq!(diff.output, Some(5));
    assert_eq!(diff.reasoning, Some(2));
    assert_eq!(quote("gpt-5.3-codex", None, &diff), Some(91_000));
    assert!(a.difference(b).is_none());
    assert_eq!(b.difference(Usage::default()).unwrap().input, None);
}
#[test]
fn mp08_mp10_mp11_agent_tree_totals_do_not_double_count_child_usage() {
    let report = from_turns(
        "session",
        vec![
            turn("p1", "root", None, 10),
            turn("p2", "child", Some("root"), 20),
            turn("p3", "other", None, 30),
        ],
    );
    assert_eq!(report.total.usage.input, Some(60));
    assert_eq!(report.agents["root"].usage.input, Some(10));
    assert_eq!(report.delegation_trees["root"].usage.input, Some(30));
    assert_eq!(report.delegation_trees["child"].usage.input, Some(20));
}
#[test]
fn mp08_mp10_mp11_durable_latest_update_survives_reopen_and_paging() {
    let dir = crate::test_support::TestWorktree::new("mp08-mp10-mp11-usage");
    let file = dir.path().join("usage.db");
    let store = OperationalHistoryStore::open(file.clone()).unwrap();
    for n in 0..502 {
        let t = turn("p1", "root", None, n);
        store
            .append_operational_event(
                crate::history::HistoryEventKind::ProviderStatus,
                None,
                None,
                BTreeMap::from([(METADATA_KEY.into(), json!(t))]),
                crate::history::HistoryEventTurnContext {
                    session_id: Some("session".into()),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    let a = load(&store, "session").unwrap();
    assert_eq!(a.turns.len(), 1);
    assert_eq!(a.total.usage.input, Some(501));
    assert_eq!(load(&store, "foreign").unwrap().turns.len(), 0);
    drop(store);
    let b = load(&OperationalHistoryStore::open(file).unwrap(), "session").unwrap();
    assert_eq!(a, b);
}
#[test]
fn mp08_mp10_mp11_unknown_price_or_counter_remains_unavailable() {
    let mut t = turn("p1", "root", None, 10);
    t.model = "gpt-6.1-sol".into();
    stamp(&mut t);
    assert_eq!(t.api_equivalent_nanodollars, None);
    t.usage = None;
    let r = from_turns("session", vec![t]);
    assert_eq!(r.total.unavailable_turns, 1);
    assert_eq!(r.total.usage.input, None);
    assert_eq!(r.total.api_equivalent_nanodollars, None);
}

#[test]
fn mp08_mp10_mp11_unmeasured_bound_prompt_does_not_disappear_from_total() {
    let dir = crate::test_support::TestWorktree::new("mp08-mp10-mp11-usage");
    let store = OperationalHistoryStore::open(dir.path().join("usage.db")).unwrap();
    store
        .append_operational_event(
            crate::history::HistoryEventKind::UserPrompt,
            None,
            None,
            BTreeMap::new(),
            crate::history::HistoryEventTurnContext {
                session_id: Some("session".into()),
                agent_id: Some("root".into()),
                prompt_id: Some("missing".into()),
                provider_run_id: Some("run".into()),
                provider: Some("codex".into()),
                model: Some("gpt-5.3-codex".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let report = load(&store, "session").unwrap();
    assert_eq!(report.total.turns, 1);
    assert_eq!(report.total.unavailable_turns, 1);
    assert_eq!(report.total.usage.input, None);
    assert_eq!(report.total.api_equivalent_nanodollars, None);
}
