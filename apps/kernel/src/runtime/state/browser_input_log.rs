//! What became of a person's input to a Tab: the last one the kernel took and the last one it
//! refused, with the reason. A card that does not take input shows nothing on the page; this is
//! what the agent that raised it reads to see why.
use super::*;

/// Tabs remembered; the one idle longest goes when a new one arrives.
const TABS: usize = 128;

/// One input, by kind only: what was typed or where it landed is never kept.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct InputOutcome {
    pub(super) at_ms: u64,
    pub(super) kind: String,
    pub(super) generation: u64,
    pub(super) document_id: String,
    /// Why it was refused; `None` for an input the kernel took.
    pub(super) reason: Option<String>,
}

#[derive(Clone, Default)]
struct TabInput {
    accepted: Option<InputOutcome>,
    refused: Option<InputOutcome>,
}

impl TabInput {
    fn last_at(&self) -> u64 {
        [&self.accepted, &self.refused]
            .into_iter()
            .flatten()
            .map(|outcome| outcome.at_ms)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Clone, Default)]
pub(super) struct BrowserInputLog(Arc<std::sync::Mutex<BTreeMap<String, TabInput>>>);

impl BrowserInputLog {
    pub(super) fn record(&self, tab_id: &str, outcome: InputOutcome) {
        let mut tabs = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tabs.contains_key(tab_id) && tabs.len() >= TABS {
            if let Some(idle) = tabs
                .iter()
                .min_by_key(|(_, input)| input.last_at())
                .map(|(tab, _)| tab.clone())
            {
                tabs.remove(&idle);
            }
        }
        let entry = tabs.entry(tab_id.to_owned()).or_default();
        if outcome.reason.is_some() {
            entry.refused = Some(outcome);
        } else {
            entry.accepted = Some(outcome);
        }
    }

    /// The last input taken and the last refused for the Tab, each with its time.
    pub(super) fn read(&self, tab_id: &str) -> (Option<InputOutcome>, Option<InputOutcome>) {
        let tabs = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        tabs.get(tab_id)
            .map(|input| (input.accepted.clone(), input.refused.clone()))
            .unwrap_or_default()
    }
}

/// The kind of a person's input (`click`, `text`, `key`, ...), as the host names it.
pub(super) fn input_kind(input: &crate::local::KernelBrowserInput) -> String {
    serde_json::to_value(input)
        .ok()
        .and_then(|value| value["kind"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "input".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(at_ms: u64, reason: Option<&str>) -> InputOutcome {
        InputOutcome {
            at_ms,
            kind: "click".into(),
            generation: 1,
            document_id: "d".into(),
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn a_tab_keeps_the_last_input_taken_and_the_last_refused_apart() {
        let log = BrowserInputLog::default();
        assert_eq!(log.read("t"), (None, None));
        log.record("t", outcome(1, None));
        log.record("t", outcome(2, Some("stale")));
        log.record("t", outcome(3, None));
        let (taken, refused) = log.read("t");
        assert_eq!(taken.unwrap().at_ms, 3);
        assert_eq!(refused.unwrap().reason.as_deref(), Some("stale"));
    }

    #[test]
    fn the_tab_idle_longest_is_forgotten_when_the_log_is_full() {
        let log = BrowserInputLog::default();
        for tab in 0..TABS as u64 {
            log.record(&format!("tab-{tab}"), outcome(tab + 1, None));
        }
        log.record("new", outcome(10_000, None));
        assert_eq!(log.read("tab-0"), (None, None), "the oldest went");
        assert!(log.read("tab-1").0.is_some());
        assert!(log.read("new").0.is_some());
    }

    #[test]
    fn an_input_is_named_by_its_kind_alone() {
        let text = crate::local::KernelBrowserInput::Text {
            text: "secret".into(),
        };
        assert_eq!(input_kind(&text), "text");
    }
}
