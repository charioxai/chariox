//! MP-08/MP-10/MP-11: shared rendered-text capture and bounded text-wait matching.
use crate::error::DaemonError;
use crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::runtime_tools::SliceBrowserTextArgs;

const MAX_WAIT_TEXT_BYTES: usize = 256 * 1024;

pub(super) async fn capture_rendered_text_page(
    state: &KernelRuntimeState,
    session_id: &str,
    tab_id: &str,
    request: SliceBrowserTextArgs,
    operation: &'static str,
) -> Result<RoomBrowserStructuredSnapshot, DaemonError> {
    let snapshot = state
        .capture_browser_environment_snapshot_with_text(session_id, tab_id, Some(request.clone()))
        .await?;
    let page = snapshot
        .text_page
        .as_ref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message:
                "Browser Controller does not support rendered text paging; upgrade the Environment"
                    .into(),
        })?;
    if page.query != request.query
        || page.offset != request.offset.min(page.total_bytes)
        || page.text.len() as u64 > request.max_bytes
    {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "Browser Controller returned a mismatched text page".into(),
        });
    }
    Ok(snapshot)
}

pub(super) async fn rendered_text_matches(
    state: &KernelRuntimeState,
    session_id: &str,
    tab_id: &str,
    query: &str,
    deadline: std::time::Instant,
) -> Result<bool, DaemonError> {
    // The controller's query selects matching lines plus their neighbors. A
    // nonempty filtered result proves a match even beyond the returned page.
    // Multiline and larger wait queries retain the existing bounded full-text
    // search, since the text-page query deliberately has a smaller limit.
    let filtered = !query.is_empty() && query.len() <= 2048 && !query.contains('\n');
    let operation = "runtime_tool_slice_browser_wait_for_text";
    let mut request = SliceBrowserTextArgs {
        query: filtered.then(|| query.to_string()),
        offset: 0,
        max_bytes: 1024,
    };
    let first =
        capture_rendered_text_page(state, session_id, tab_id, request.clone(), operation).await?;
    let first_page = first.text_page.as_ref().expect("validated text page");
    if filtered {
        return Ok(first_page.total_bytes > 0);
    }
    let total_bytes = first_page.total_bytes;
    let binding = (
        first.environment_id.clone(),
        first.runtime_generation,
        first.browser_generation,
        first.document_revision,
    );
    let mut snapshot = first;
    let mut text = String::new();
    loop {
        // Pages are live observations. Never join across a document/runtime
        // replacement or a changed length; retry through the normal wait loop.
        if snapshot.environment_id != binding.0
            || snapshot.runtime_generation != binding.1
            || snapshot.browser_generation != binding.2
            || snapshot.document_revision != binding.3
        {
            return Ok(false);
        }
        let page = snapshot.text_page.as_ref().expect("validated text page");
        if page.total_bytes != total_bytes {
            return Ok(false);
        }
        let mut end = page.text.len().min(MAX_WAIT_TEXT_BYTES - text.len());
        while !page.text.is_char_boundary(end) {
            end -= 1;
        }
        text.push_str(&page.text[..end]);
        if text.contains(query) {
            return Ok(true);
        }
        let Some(offset) = page.next_offset else {
            return Ok(false);
        };
        if end < page.text.len()
            || text.len() >= MAX_WAIT_TEXT_BYTES
            || std::time::Instant::now() >= deadline
        {
            return Ok(false);
        }
        request.offset = offset;
        snapshot =
            capture_rendered_text_page(state, session_id, tab_id, request.clone(), operation)
                .await?;
    }
}
