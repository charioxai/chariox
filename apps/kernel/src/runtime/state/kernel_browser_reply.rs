//! Row 552: what an agent's kernel_browser action returns about the tabs. The host answers every
//! action with the whole tab list and each tab's favicon (about 9k tokens with 14 tabs open); an
//! agent needs the acted tab and, only when tabs opened or closed, that change. The full list is
//! the answer to an explicit `state` (or `start`) read, still without the icon bytes.
use serde_json::Value;
use std::collections::BTreeSet;

/// Tab ids an agent was last told about, kept per agent by the caller.
pub(super) type SeenTabs = BTreeSet<String>;

/// Rewrites the host's reply for the agent. `op` is the command's op, `acting_tab` its `tab_id`.
/// A read of the state (or a start) lists every tab; any other action answers with the acted tab,
/// the tab count and, only when tabs opened or closed since the agent last heard, that change.
/// Icons never reach the agent.
pub(super) fn slim_agent_reply(
    op: &str,
    acting_tab: Option<&str>,
    mut payload: Value,
    seen: &mut SeenTabs,
) -> Value {
    let acting = acting_tab
        .map(str::to_owned)
        .or_else(|| payload["tab_id"].as_str().map(str::to_owned));
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    let Some(Value::Array(mut tabs)) = object.remove("tabs") else {
        return payload;
    };
    for tab in tabs.iter_mut().filter_map(Value::as_object_mut) {
        tab.remove("favicon_png_base64");
        tab.remove("favicon_hash");
    }
    let id = |tab: &Value| tab["tab_id"].as_str().map(str::to_owned);
    let ids: SeenTabs = tabs.iter().filter_map(id).collect();
    if matches!(op, "state" | "start") {
        object.insert("tabs".into(), Value::Array(tabs));
        *seen = ids;
        return payload;
    }
    object.insert("tab_count".into(), tabs.len().into());
    if let Some(tab) = tabs.iter().find(|tab| id(tab) == acting) {
        object.insert("tab".into(), tab.clone());
    }
    if !seen.is_empty() {
        let opened: Vec<Value> = tabs
            .iter()
            .filter(|tab| id(tab).is_some_and(|id| !seen.contains(&id)))
            .cloned()
            .collect();
        let closed: Vec<&String> = seen.difference(&ids).collect();
        if !opened.is_empty() || !closed.is_empty() {
            object.insert(
                "tabs_changed".into(),
                serde_json::json!({"opened": opened, "closed": closed}),
            );
        }
    }
    *seen = ids;
    payload
}

/// Runs `reply` against the tabs this agent was last told about.
pub(super) fn slim_for_agent(
    agent_id: &str,
    op: &str,
    acting_tab: Option<&str>,
    payload: Value,
) -> Value {
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, SeenTabs>>,
    > = std::sync::OnceLock::new();
    let mut by_agent = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // One small set per agent; a kernel with a very long agent list starts over rather than grow.
    if by_agent.len() > 1024 {
        by_agent.clear();
    }
    let seen = by_agent.entry(agent_id.to_owned()).or_default();
    slim_agent_reply(op, acting_tab, payload, seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(index: usize) -> Value {
        json!({"tab_id":format!("host-tab-{index}"),"document_id":format!("doc-{index}"),
            "title":format!("Tab {index}"),"url":format!("https://site{index}.example.com/inbox"),
            "favicon_png_base64":"iVBORw0KGgo".repeat(400),"favicon_hash":"0011223344556677"})
    }

    fn reply(count: usize) -> Value {
        json!({"generation":4,"state":"running","tabs":(0..count).map(tab).collect::<Vec<_>>(),
            "viewport":{"css_width":1280,"css_height":800},"document_id":"doc-3"})
    }

    /// ~4 characters per token: the cell's "under about 1k tokens" is 4 KB of reply text.
    const ONE_K_TOKENS: usize = 4_000;

    #[test]
    fn twenty_clicks_in_one_tab_with_fourteen_open_each_reply_under_one_k_tokens() {
        let mut seen = SeenTabs::new();
        // The first reply of an agent sets its baseline.
        let first = slim_agent_reply("input", Some("host-tab-3"), reply(14), &mut seen);
        assert!(
            first.to_string().len() < ONE_K_TOKENS,
            "{}",
            first.to_string().len()
        );
        for _ in 0..20 {
            let slim = slim_agent_reply("input", Some("host-tab-3"), reply(14), &mut seen);
            let text = slim.to_string();
            assert!(text.len() < ONE_K_TOKENS, "{} bytes", text.len());
            assert!(!text.contains("favicon"), "icons never reach the agent");
            assert!(
                slim.get("tabs_changed").is_none(),
                "nothing opened or closed"
            );
            assert_eq!(slim["tab"]["tab_id"], "host-tab-3");
            assert_eq!(slim["tab_count"], 14);
            assert_eq!(slim["generation"], 4);
        }
    }

    #[test]
    fn a_popup_shows_as_one_added_tab_and_a_close_as_one_removed() {
        let mut seen = SeenTabs::new();
        slim_agent_reply("input", Some("host-tab-3"), reply(14), &mut seen);
        let opened = slim_agent_reply("input", Some("host-tab-3"), reply(15), &mut seen);
        let added = opened["tabs_changed"]["opened"].as_array().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["tab_id"], "host-tab-14");
        assert!(added[0].get("favicon_png_base64").is_none());
        assert_eq!(opened["tabs_changed"]["closed"], json!([]));
        assert_eq!(opened["tab_count"], 15);
        // The next action reports it once, not again.
        let quiet = slim_agent_reply("input", Some("host-tab-3"), reply(15), &mut seen);
        assert!(quiet.get("tabs_changed").is_none());
        let closed = slim_agent_reply("close", Some("host-tab-14"), reply(14), &mut seen);
        assert_eq!(closed["tabs_changed"]["closed"], json!(["host-tab-14"]));
    }

    #[test]
    fn an_explicit_state_read_lists_every_tab_without_icon_bytes() {
        let mut seen = SeenTabs::new();
        let state = slim_agent_reply("state", None, reply(14), &mut seen);
        assert_eq!(state["tabs"].as_array().unwrap().len(), 14);
        assert!(!state.to_string().contains("favicon"));
        // A read also sets the baseline: a later click reports only changes after it.
        let click = slim_agent_reply("input", Some("host-tab-0"), reply(14), &mut seen);
        assert!(click.get("tabs_changed").is_none());
    }

    #[test]
    fn a_reply_without_tabs_or_the_acting_tab_is_left_alone() {
        let mut seen = SeenTabs::new();
        let image = json!({"generation":2,"image_base64":"AAAA","tab_id":"t"});
        assert_eq!(
            slim_agent_reply("screenshot", Some("t"), image.clone(), &mut seen),
            image
        );
        let gone = slim_agent_reply("input", Some("missing"), reply(3), &mut seen);
        assert_eq!(gone["tab_count"], 3);
        assert!(gone.get("tab").is_none());
    }
    #[test]
    fn a_start_a_restart_and_a_stop_answer_with_the_whole_tab_list_like_a_state_read() {
        for op in ["state", "start", "restart", "stop"] {
            let mut seen = SeenTabs::new();
            let out = slim_agent_reply(op, None, reply(3), &mut seen);
            assert_eq!(
                out["tabs"].as_array().map(Vec::len),
                Some(3),
                "{op} lists every tab"
            );
            assert!(
                out.get("tab_count").is_none(),
                "{op} is not a compact action reply"
            );
        }
        let mut seen = SeenTabs::new();
        let stopped = slim_agent_reply(
            "stop",
            None,
            json!({"state":"stopped","tabs":[]}),
            &mut seen,
        );
        assert_eq!(
            stopped["tabs"],
            json!([]),
            "a stop keeps its empty tab list"
        );
        for op in ["input", "open", "navigate", "close", "wait"] {
            let mut seen = SeenTabs::new();
            let out = slim_agent_reply(op, Some("host-tab-1"), reply(3), &mut seen);
            assert!(
                out.get("tabs").is_none(),
                "{op} stays a compact action reply"
            );
        }
    }
}
