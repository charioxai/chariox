use std::path::PathBuf;

pub(crate) fn artifact_attachment_segment(attachment_id: &str) -> String {
    attachment_id
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect()
}

pub(crate) fn attachment_artifact_root(
    session_id: &str,
    attachment_id: &str,
    category: &str,
) -> PathBuf {
    std::env::temp_dir()
        .join("arroba-session-artifacts")
        .join(session_id)
        .join(category)
        .join(artifact_attachment_segment(attachment_id))
}

pub(crate) fn attachment_artifact_roots(session_id: &str, attachment_id: &str) -> [PathBuf; 2] {
    [
        attachment_artifact_root(session_id, attachment_id, "screenshots"),
        attachment_artifact_root(session_id, attachment_id, "transfers"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_attachment_segment_replaces_path_unsafe_characters() {
        assert_eq!(
            artifact_attachment_segment("client/one:bad*name?\n"),
            "client_one_bad_name__"
        );
    }

    #[test]
    fn attachment_artifact_roots_are_scoped_by_session_category_and_attachment() {
        let roots = attachment_artifact_roots("session-1", "client/one");

        assert!(roots[0].ends_with("session-1/screenshots/client_one"));
        assert!(roots[1].ends_with("session-1/transfers/client_one"));
    }
}
