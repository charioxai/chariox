//! The tracked, individually approved card list shared with clients.
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct Registration {
    #[serde(rename = "type")]
    name: String,
    class: String,
}

#[derive(serde::Deserialize)]
struct Decision {
    id: String,
    approved_by: String,
    source: String,
    types: Vec<Registration>,
}

fn approved(entry: &Registration, decisions: &[Decision]) -> bool {
    matches!(
        entry.class.as_str(),
        "asked-for notification" | "person action or answer" | "live browser card"
    ) && decisions.iter().any(|decision| {
        decision.approved_by == "Miguel"
            && !decision.id.trim().is_empty()
            && !decision.source.trim().is_empty()
            && decision
                .types
                .iter()
                .any(|pair| pair.name == entry.name && pair.class == entry.class)
    })
}

pub fn runtime_card_class(name: &str) -> Option<&'static str> {
    static TYPES: OnceLock<Vec<Registration>> = OnceLock::new();
    TYPES
        .get_or_init(|| {
            let decisions: Vec<Decision> =
                serde_json::from_str(include_str!("../../../../docs/card-decisions.json"))
                    .expect("the tracked runtime card decisions must be valid");
            let entries: Vec<Registration> =
                serde_json::from_str(include_str!("../../../../docs/runtime-card-types.json"))
                    .expect("the tracked runtime card list must be valid");
            entries
                .into_iter()
                .filter(|entry| {
                    let allowed = approved(entry, &decisions);
                    if !allowed {
                        tracing::error!(
                            card_type = entry.name,
                            class = entry.class,
                            "card refused: missing individual type/class decision"
                        );
                    }
                    allowed
                })
                .collect()
        })
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.class.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_individual_registered_types_have_a_class() {
        assert_eq!(
            runtime_card_class("Question"),
            Some("person action or answer")
        );
        assert_eq!(runtime_card_class("unknown-test-card"), None);
        assert_eq!(runtime_card_class("Task"), None);
        assert_eq!(runtime_card_class("OwnerControl"), None);
    }
}
