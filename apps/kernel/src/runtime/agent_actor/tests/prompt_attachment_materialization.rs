use super::*;

#[test]
fn materializes_inline_prompt_attachments_to_local_files() {
    let attachment = PromptAttachment::new(
        "chariox-cloud://artifact/art-1",
        "text/plain",
        Some("../note.txt".to_string()),
    )
    .with_contents_base64(base64::engine::general_purpose::STANDARD.encode("hello artifact"));

    let materialized =
        materialize_inline_prompt_attachments("session/one", "agent:one", vec![attachment])
            .expect("inline attachment should materialize");

    assert_eq!(materialized.len(), 1);
    let path = materialized[0]
        .url()
        .strip_prefix("file://")
        .expect("materialized attachment should use file URL");
    assert_eq!(materialized[0].mime(), "text/plain");
    assert_eq!(materialized[0].filename(), Some("note.txt"));
    assert_eq!(
        fs::read_to_string(path).expect("materialized file should be readable"),
        "hello artifact"
    );
    assert!(path.contains("session-one"));
    assert!(path.contains("agent-one"));
    assert!(path.contains(INLINE_PROMPT_ATTACHMENT_DIR));
}
