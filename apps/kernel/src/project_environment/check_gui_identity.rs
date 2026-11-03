//! MP-08 / MP-10 / MP-11: GUI identities are probe policy, not filesystem paths.
pub(super) fn application_name(identity: &str) -> Option<&str> {
    match identity.to_ascii_lowercase().as_str() {
        "chatgpt" | "chatgpt.app" => Some("ChatGPT"),
        "notion" | "notion.app" => Some("Notion"),
        "canva" | "canva.app" => Some("Canva"),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_probe_rejects_paths_and_unsupported_application_names() {
        for identity in [
            "../private.app",
            "/private/secret.app",
            "subdir/secret.app",
            "Unknown.app",
            "../Notion.app",
        ] {
            assert_eq!(application_name(identity), None, "{identity}");
        }
    }
    #[test]
    fn gui_probe_keeps_named_case_insensitive_applications() {
        for (identity, expected) in [
            ("ChatGPT", "ChatGPT"),
            ("NOTION", "Notion"),
            ("canva", "Canva"),
        ] {
            assert_eq!(application_name(identity), Some(expected));
        }
    }
}
