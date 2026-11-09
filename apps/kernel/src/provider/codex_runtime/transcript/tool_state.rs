use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::terminal::TerminalOutputKind;

use super::item::is_codex_tool_item;
use super::tool_update::render_codex_tool_transcript_update;
use crate::extension::RemoteExtensionManifest;
use crate::provider::codex_runtime::CodexOutputChunk;
use crate::runtime::workspace_repo_files::{read_workspace_picture, WorkspacePicture};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(in crate::provider::codex_runtime) struct CodexToolTranscriptState {
    pub(in crate::provider::codex_runtime) item: Value,
    pub(in crate::provider::codex_runtime) streamed_output: String,
    pub(in crate::provider::codex_runtime) pending_output_bytes: Vec<u8>,
    pub(in crate::provider::codex_runtime) output_stream: Option<ToolOutputStream>,
    pub(in crate::provider::codex_runtime) progress_messages: Vec<String>,
    pub(in crate::provider::codex_runtime) last_emitted: Option<String>,
    /// What a completed `view_image` call showed, read once when the call completes.
    pub(in crate::provider::codex_runtime) pictures: Option<CodexToolPictures>,
    /// When the kernel first saw the call.
    pub(in crate::provider::codex_runtime) started_at_ms: u64,
    /// A command the agent left running past its call: polled through its
    /// terminal, or still in progress when its turn completed.
    pub(in crate::provider::codex_runtime) background: bool,
    pub(in crate::provider::codex_runtime) finished_at_ms: Option<u64>,
}

impl CodexToolTranscriptState {
    pub(in crate::provider::codex_runtime) fn new(item: Value, now_ms: u64) -> Self {
        Self {
            item,
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
            pictures: None,
            started_at_ms: now_ms,
            background: false,
            finished_at_ms: None,
        }
    }

    fn running_command(&self) -> bool {
        self.item.get("type").and_then(Value::as_str) == Some("commandExecution")
            && self.item.get("status").and_then(Value::as_str) == Some("inProgress")
    }
}

/// The command keeps running after the agent moved on: from now on its call
/// shows as a background command until Codex reports its exit.
pub(in crate::provider::codex_runtime) fn mark_background_command(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
) -> Option<CodexOutputChunk> {
    let entry = tool_items.get_mut(item_id)?;
    if entry.background || !entry.running_command() {
        return None;
    }
    entry.background = true;
    render_tool_chunk_if_changed(item_id, entry, &RemoteExtensionManifest::default())
}

/// Every command still running when its turn completed is a background one.
pub(in crate::provider::codex_runtime) fn mark_running_commands_background(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
) -> Vec<CodexOutputChunk> {
    let running = tool_items
        .iter()
        .filter(|(_, entry)| !entry.background && entry.running_command())
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    running
        .iter()
        .filter_map(|id| mark_background_command(tool_items, id))
        .collect()
}

pub(in crate::provider::codex_runtime) fn is_running_background_command(
    tool_items: &BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
) -> bool {
    tool_items
        .get(item_id)
        .is_some_and(|entry| entry.background && entry.finished_at_ms.is_none())
}


/// The pictures a call carries to the transcript, and how many were too large to keep.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(in crate::provider::codex_runtime) struct CodexToolPictures {
    pub(in crate::provider::codex_runtime) images: Vec<String>,
    pub(in crate::provider::codex_runtime) omitted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::provider::codex_runtime) enum ToolOutputStream {
    Legacy,
    Modern,
}

pub(in crate::provider::codex_runtime) fn codex_exec_command_item(
    call_id: &str,
    command: Value,
    cwd: Option<String>,
    exit_code: Option<i64>,
) -> Value {
    let mut item = json!({
        "id": call_id,
        "type": "commandExecution",
        "status": "inProgress",
        "command": normalize_codex_command_value(&command),
    });
    if let Some(cwd) = cwd {
        item["cwd"] = json!(cwd);
    }
    if let Some(exit_code) = exit_code {
        item["exitCode"] = json!(exit_code);
    }
    item
}

fn normalize_codex_command_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(in crate::provider::codex_runtime) fn decode_codex_output_delta_chunk(chunk: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(chunk)
        .unwrap_or_else(|_| chunk.as_bytes().to_vec())
}

/// The event an item arrived with. Web searches and image views carry no status of their own, so this decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::provider::codex_runtime) enum CodexItemLifecycle {
    Started,
    /// Also the lifecycle of items read back from a finished turn.
    Completed,
}

pub(in crate::provider::codex_runtime) fn sync_tool_item_with_manifest(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item: &Value,
    lifecycle: CodexItemLifecycle,
    remote_extension_manifest: &RemoteExtensionManifest,
    workspace_root: Option<&Path>,
) -> Option<CodexOutputChunk> {
    if !is_codex_tool_item(item) {
        return None;
    }
    let now_ms = crate::session::unix_epoch_ms();
    let item_id = item.get("id").and_then(Value::as_str)?.to_string();
    let mut item = item.clone();
    if matches!(
        item.get("type").and_then(Value::as_str),
        Some("webSearch" | "imageView" | "imageGeneration")
    ) && item.get("status").is_none()
    {
        item["status"] = json!(match lifecycle {
            CodexItemLifecycle::Started => "inProgress",
            CodexItemLifecycle::Completed => "completed",
        });
    }
    let item = &item;
    let entry = tool_items
        .entry(item_id.clone())
        .or_insert_with(|| CodexToolTranscriptState::new(item.clone(), now_ms));
    entry.item = item.clone();
    if entry.background
        && entry.finished_at_ms.is_none()
        && lifecycle == CodexItemLifecycle::Completed
    {
        entry.finished_at_ms = Some(now_ms);
    }
    if entry.pictures.is_none() && is_completed_image_view(item) {
        entry.pictures = Some(viewed_pictures(item, workspace_root));
    }
    for field in ["aggregatedOutput", "aggregated_output", "output", "stderr"] {
        if let Some(serde_json::Value::String(text)) = entry.item.get_mut(field) {
            super::output_window::trim(text);
        }
    }
    if matches!(
        item.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "cancelled" | "canceled" | "interrupted")
    ) && !entry.pending_output_bytes.is_empty()
    {
        entry
            .streamed_output
            .push_str(&String::from_utf8_lossy(&entry.pending_output_bytes));
        entry.pending_output_bytes.clear();
    }
    super::output_window::trim(&mut entry.streamed_output);
    render_tool_chunk_if_changed(&item_id, entry, remote_extension_manifest)
}

fn is_completed_image_view(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("imageView")
        && item.get("status").and_then(Value::as_str) == Some("completed")
}

/// The picture a completed `view_image` call showed, when it is an image file the kernel may read for the agent.
fn viewed_pictures(item: &Value, workspace_root: Option<&Path>) -> CodexToolPictures {
    let mut pictures = CodexToolPictures::default();
    let Some((workspace_root, path)) =
        workspace_root.zip(item.get("path").and_then(Value::as_str).and_then(non_empty))
    else {
        return pictures;
    };
    match read_workspace_picture(
        workspace_root,
        Path::new(path),
        crate::provider::RESULT_IMAGE_FILE_BYTES,
    ) {
        Some(WorkspacePicture::Picture { media_type, bytes }) => {
            let block = json!([{
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            }]);
            (pictures.images, pictures.omitted) =
                crate::provider::bounded_result_images(Some(&block));
        }
        Some(WorkspacePicture::TooLarge) => pictures.omitted = 1,
        None => {}
    }
    pictures
}

pub(in crate::provider::codex_runtime) fn append_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &str,
) -> Option<CodexOutputChunk> {
    append_output_delta_from_stream(
        tool_items,
        item_id,
        item_type,
        delta.as_bytes(),
        ToolOutputStream::Modern,
    )
}

pub(in crate::provider::codex_runtime) fn append_legacy_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &[u8],
) -> Option<CodexOutputChunk> {
    append_output_delta_from_stream(
        tool_items,
        item_id,
        item_type,
        delta,
        ToolOutputStream::Legacy,
    )
}

fn append_output_delta_from_stream(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &[u8],
    stream: ToolOutputStream,
) -> Option<CodexOutputChunk> {
    if delta.is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| {
            CodexToolTranscriptState::new(
                placeholder_tool_item(item_id, item_type),
                crate::session::unix_epoch_ms(),
            )
        });
    // MP-08 / MP-10: app-server can emit both legacy and modern deltas for
    // one call. Keep the first live stream, avoiding duplicated bytes while
    // preserving the prefix delivered before the modern stream starts.
    if entry
        .output_stream
        .is_some_and(|selected| selected != stream)
    {
        return None;
    }
    entry.output_stream = Some(stream);
    entry.pending_output_bytes.extend_from_slice(delta);
    let mut consumed = 0;
    while consumed < entry.pending_output_bytes.len() {
        let remaining = &entry.pending_output_bytes[consumed..];
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                entry.streamed_output.push_str(text);
                consumed = entry.pending_output_bytes.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                entry
                    .streamed_output
                    .push_str(std::str::from_utf8(&remaining[..valid]).unwrap());
                consumed += valid;
                if let Some(invalid) = error.error_len() {
                    entry.streamed_output.push('\u{fffd}');
                    consumed += invalid;
                } else {
                    // Retain only an incomplete trailing code point for this
                    // call; another tool's chunks cannot complete it.
                    break;
                }
            }
        }
    }
    entry.pending_output_bytes.drain(..consumed);
    if entry.pending_output_bytes.capacity() > super::output_window::MAX_OUTPUT_BYTES * 2 {
        entry.pending_output_bytes.shrink_to_fit();
    }
    super::output_window::trim(&mut entry.streamed_output);
    render_tool_chunk_if_changed(item_id, entry, &RemoteExtensionManifest::default())
}

pub(in crate::provider::codex_runtime) fn append_tool_progress(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    message: &str,
) -> Option<CodexOutputChunk> {
    if message.trim().is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| {
            CodexToolTranscriptState::new(
                placeholder_tool_item(item_id, "mcpToolCall"),
                crate::session::unix_epoch_ms(),
            )
        });
    entry.progress_messages.push(message.trim().to_string());
    render_tool_chunk_if_changed(item_id, entry, &RemoteExtensionManifest::default())
}

fn render_tool_chunk_if_changed(
    item_id: &str,
    state: &mut CodexToolTranscriptState,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> Option<CodexOutputChunk> {
    let rendered = render_codex_tool_transcript_update(state, remote_extension_manifest)?;
    if state.last_emitted.as_deref() == Some(rendered.as_str()) {
        return None;
    }
    state.last_emitted = Some(rendered.clone());
    Some(CodexOutputChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: Some(item_id.to_string()),
        bytes: rendered.into_bytes(),
    })
}

fn placeholder_tool_item(item_id: &str, item_type: &str) -> Value {
    json!({
        "id": item_id,
        "type": item_type,
        "status": "inProgress",
    })
}

pub(super) fn normalize_merge_key(item_id: &str, fallback: &str) -> String {
    non_empty(item_id).unwrap_or(fallback).to_string()
}

pub(super) fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod output_stream_tests {
    use super::*;
    // MP-08 / MP-10: reviewer #985 finding 2. The transport splits a
    // three-byte character, so decoding each chunk as a String is invalid.
    #[test]
    fn panes_stream_review_legacy_split_utf8_stays_text() {
        let mut states = BTreeMap::new();
        for bytes in [&"✓".as_bytes()[..1], &"✓".as_bytes()[1..]] {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let decoded = decode_codex_output_delta_chunk(&encoded);
            append_legacy_tool_output_delta(&mut states, "unicode", "commandExecution", &decoded);
        }
        assert_eq!(states["unicode"].streamed_output, "✓");
    }

    #[test]
    fn legacy_utf8_carry_is_per_call_and_invalid_bytes_are_lossy() {
        let mut states = BTreeMap::new();
        append_legacy_tool_output_delta(
            &mut states,
            "one",
            "commandExecution",
            &"✓".as_bytes()[..1],
        );
        append_legacy_tool_output_delta(&mut states, "two", "commandExecution", &[0xff]);
        append_legacy_tool_output_delta(
            &mut states,
            "one",
            "commandExecution",
            &"✓".as_bytes()[1..],
        );
        assert_eq!(states["one"].streamed_output, "✓");
        assert_eq!(states["two"].streamed_output, "\u{fffd}");
        assert!(states["one"].pending_output_bytes.is_empty());
    }
    #[test]
    fn legacy_prefix_and_modern_mirror_do_not_duplicate_streamed_tool_output() {
        let mut state = BTreeMap::new();
        let first =
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"line 1\n")
                .unwrap();
        assert!(
            append_tool_output_delta(&mut state, "call", "commandExecution", "line 1\n").is_none()
        );
        let second =
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"line 2\n")
                .unwrap();
        let first: Value = serde_json::from_slice(&first.bytes).unwrap();
        let second: Value = serde_json::from_slice(&second.bytes).unwrap();
        assert_eq!(first["output"], "line 1");
        assert_eq!(second["output"], "line 1\nline 2");
    }
    #[test]
    fn modern_only_and_late_legacy_mirror_keep_the_existing_stream() {
        let mut state = BTreeMap::new();
        append_tool_output_delta(&mut state, "call", "commandExecution", "first ").unwrap();
        assert!(
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"first ")
                .is_none()
        );
        let second =
            append_tool_output_delta(&mut state, "call", "commandExecution", "second").unwrap();
        let second: Value = serde_json::from_slice(&second.bytes).unwrap();
        assert_eq!(second["output"], "first second");
    }
}

#[cfg(test)]
mod output_window_tests {
    use super::*;

    #[test]
    fn streamed_and_completed_tool_output_keep_a_bounded_utf8_tail() {
        let mut tools = BTreeMap::new();
        for _ in 0..100 {
            let delta = "λ".repeat(10_000);
            append_tool_output_delta(&mut tools, "flood", "commandExecution", &delta);
        }
        let chunk = append_tool_output_delta(&mut tools, "flood", "commandExecution", "FINAL_TAIL")
            .unwrap();
        let rendered: Value = serde_json::from_slice(&chunk.bytes).unwrap();
        let output = rendered["output"].as_str().unwrap();
        assert!(output.len() <= super::super::output_window::MAX_OUTPUT_BYTES);
        assert!(output.starts_with("[earlier tool output omitted]"));
        assert!(output.ends_with("FINAL_TAIL"));
        assert!(
            tools["flood"].streamed_output.capacity()
                <= super::super::output_window::MAX_OUTPUT_BYTES * 2
        );
        let completed = json!({"id":"flood", "type":"commandExecution", "status":"completed", "aggregatedOutput":format!("{}COMPLETE_TAIL", "x".repeat(2_000_000))});
        let chunk = sync_tool_item_with_manifest(
            &mut tools,
            &completed,
            CodexItemLifecycle::Completed,
            &RemoteExtensionManifest::default(),
            None,
        )
        .unwrap();
        let rendered: Value = serde_json::from_slice(&chunk.bytes).unwrap();
        let output = rendered["output"].as_str().unwrap();
        assert!(output.len() <= super::super::output_window::MAX_OUTPUT_BYTES);
        assert!(output.starts_with("[earlier tool output omitted]"));
        assert!(output.ends_with("COMPLETE_TAIL"));
        assert!(
            tools["flood"].item["aggregatedOutput"]
                .as_str()
                .unwrap()
                .len()
                <= super::super::output_window::MAX_OUTPUT_BYTES
        );
    }
}

#[cfg(test)]
mod view_image_tests {
    use super::*;

    // A 1x1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("chariox-view-image-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("workspace")).unwrap();
            std::fs::create_dir_all(root.join("elsewhere")).unwrap();
            Self(root)
        }

        fn workspace(&self) -> std::path::PathBuf {
            self.0.join("workspace")
        }

        fn elsewhere(&self) -> std::path::PathBuf {
            self.0.join("elsewhere")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn view(path: &Path, lifecycle: CodexItemLifecycle, workspace: &Path) -> Value {
        let item = json!({"type": "imageView", "id": "iv-1", "path": path});
        let mut tools = BTreeMap::new();
        let chunk = sync_tool_item_with_manifest(
            &mut tools,
            &item,
            lifecycle,
            &RemoteExtensionManifest::default(),
            Some(workspace),
        )
        .expect("a view_image update");
        serde_json::from_slice(&chunk.bytes).unwrap()
    }

    fn completed_view(path: &Path, workspace: &Path) -> Value {
        view(path, CodexItemLifecycle::Completed, workspace)
    }

    #[test]
    fn a_viewed_picture_in_the_workspace_is_carried_with_its_call() {
        let scratch = Scratch::new("picture");
        std::fs::write(scratch.workspace().join("chart.png"), PNG).unwrap();
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(PNG)
        );
        let absolute = completed_view(&scratch.workspace().join("chart.png"), &scratch.workspace());
        assert_eq!(absolute["tool"], "view_image");
        assert_eq!(absolute["images"], json!([expected]));
        assert!(absolute.get("images_omitted").is_none());
        let relative = completed_view(Path::new("chart.png"), &scratch.workspace());
        assert_eq!(relative["images"], json!([expected]));
        let running = view(
            &scratch.workspace().join("chart.png"),
            CodexItemLifecycle::Started,
            &scratch.workspace(),
        );
        assert!(
            running.get("images").is_none(),
            "no picture before the call completes"
        );
    }

    #[test]
    fn a_picture_is_recognised_by_its_content_not_its_name() {
        let scratch = Scratch::new("content");
        std::fs::write(scratch.workspace().join("figure"), PNG).unwrap();
        std::fs::write(scratch.workspace().join("notes.png"), b"just text").unwrap();
        std::fs::write(scratch.workspace().join("vector.svg"), b"<svg/>").unwrap();
        let named_oddly = completed_view(&scratch.workspace().join("figure"), &scratch.workspace());
        assert_eq!(named_oddly["images"].as_array().map(Vec::len), Some(1));
        for not_a_picture in ["notes.png", "vector.svg", "missing.png"] {
            let rendered = completed_view(
                &scratch.workspace().join(not_a_picture),
                &scratch.workspace(),
            );
            assert!(rendered.get("images").is_none(), "{not_a_picture}");
            assert!(rendered.get("images_omitted").is_none(), "{not_a_picture}");
        }
    }

    #[test]
    fn a_picture_outside_the_workspace_is_never_read() {
        let scratch = Scratch::new("outside");
        std::fs::write(scratch.elsewhere().join("secret.png"), PNG).unwrap();
        let outside = completed_view(
            &scratch.elsewhere().join("secret.png"),
            &scratch.workspace(),
        );
        assert!(outside.get("images").is_none());
        let traversal = completed_view(Path::new("../elsewhere/secret.png"), &scratch.workspace());
        assert!(traversal.get("images").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                scratch.elsewhere().join("secret.png"),
                scratch.workspace().join("link.png"),
            )
            .unwrap();
            let linked =
                completed_view(&scratch.workspace().join("link.png"), &scratch.workspace());
            assert!(linked.get("images").is_none(), "a symlink does not escape");
        }
    }

    #[test]
    fn a_picture_over_the_size_limit_is_counted_as_omitted() {
        let scratch = Scratch::new("large");
        let mut large = PNG.to_vec();
        large.resize(crate::provider::RESULT_IMAGE_FILE_BYTES + 1, 0);
        std::fs::write(scratch.workspace().join("big.png"), &large).unwrap();
        let rendered = completed_view(&scratch.workspace().join("big.png"), &scratch.workspace());
        assert!(rendered.get("images").is_none());
        assert_eq!(rendered["images_omitted"], 1);
        large.truncate(crate::provider::RESULT_IMAGE_FILE_BYTES);
        std::fs::write(scratch.workspace().join("fits.png"), &large).unwrap();
        let fits = completed_view(&scratch.workspace().join("fits.png"), &scratch.workspace());
        assert_eq!(fits["images"].as_array().map(Vec::len), Some(1));
    }
}
