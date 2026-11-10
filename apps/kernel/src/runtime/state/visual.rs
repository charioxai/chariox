//! Pages an agent shows the person: `chariox.visual.show` keeps a copy with the Room for the client to draw above the
//! reply; `chariox.visual.preview` renders a page so the agent can check it first (see `visual_preview`).
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use sha2::{Digest, Sha256};
use std::io::Read;

pub(crate) const VISUAL_SHOW: &str = "chariox.visual.show";
pub(crate) const VISUAL_PREVIEW: &str = "chariox.visual.preview";
/// A Room view reads a page of at most this size.
pub(super) const MAX_PAGE_BYTES: u64 = 1024 * 1024;
const PAGE_RULES: &str = "A page is one self-contained HTML document (inline <style> and <script>; scripts may load only from cdn.jsdelivr.net, cdnjs.cloudflare.com, unpkg.com or esm.sh, and the page cannot fetch data, so its data is in the page), at most 1 MB.";

pub(crate) fn is_visual_tool(name: &str) -> bool {
    canonical(name).is_some()
}

fn canonical(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("mcp__chariox__").unwrap_or(name);
    [VISUAL_SHOW, VISUAL_PREVIEW]
        .into_iter()
        .find(|tool| name == *tool || name == tool.replace('.', "_"))
}

pub(crate) fn visual_tool_specs() -> Vec<RuntimeToolSpec> {
    let source = serde_json::json!({
        "path": {"type": "string", "description": "Full path of an .html file, or one relative to your working directory"},
        "html": {"type": "string", "description": "The page itself, instead of a path"}
    });
    let with = |extra: serde_json::Value| {
        let mut properties = source.as_object().cloned().unwrap_or_default();
        properties.extend(extra.as_object().cloned().unwrap_or_default());
        serde_json::json!({"type": "object", "properties": properties, "additionalProperties": false})
    };
    vec![
        RuntimeToolSpec {
            name: VISUAL_SHOW.into(),
            description: format!("Show the person an interactive chart, diagram or table as a page drawn above your reply in the conversation, in its light or dark colors. Give a path or the page itself (one of the two). Kernel keeps a copy, so the page stays when the file moves. Preview it first. Do not mention or describe the page in your reply, and do not paste its source. {PAGE_RULES}"),
            input_schema: with(serde_json::json!({"wide": {"type": "boolean", "description": "Draw the page across the full width of the pane"}})),
        },
        RuntimeToolSpec {
            name: VISUAL_PREVIEW.into(),
            description: format!("Render a page the way the conversation will draw it and return a picture of it, its content height and what its console printed, so you can check it before showing it. Give a path or the page itself (one of the two). {PAGE_RULES}"),
            input_schema: with(serde_json::json!({"width": {"type": "integer", "minimum": 320, "maximum": 1600, "default": 900}})),
        },
    ]
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ShowArgs {
    path: Option<String>,
    html: Option<String>,
    #[serde(default)]
    wide: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewArgs {
    path: Option<String>,
    html: Option<String>,
    width: Option<u32>,
}

fn refusal(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "visual",
        message: message.into(),
    }
}

/// The page named by exactly one of `path` and `html`, as text no larger than a Room view reads.
fn page_text(
    path: Option<String>,
    html: Option<String>,
    working_directory: Option<&std::path::Path>,
) -> Result<String, DaemonError> {
    let page = match (path, html) {
        (Some(path), None) => {
            let path = std::path::PathBuf::from(path.trim());
            let path = match (path.is_absolute(), working_directory) {
                (true, _) => path,
                (false, Some(directory)) => directory.join(path),
                (false, None) => {
                    return Err(refusal(
                        "give the full path of the page, or pass the page as html",
                    ))
                }
            };
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(extension.as_deref(), Some("html" | "htm")) {
                return Err(refusal(format!("{} is not an .html file", path.display())));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .and_then(|file| file.take(MAX_PAGE_BYTES + 1).read_to_end(&mut bytes))
                .map_err(|error| refusal(format!("cannot read {}: {error}", path.display())))?;
            String::from_utf8(bytes).map_err(|_| refusal("the page is not UTF-8 text"))?
        }
        (None, Some(html)) => html,
        _ => {
            return Err(refusal(
                "give the page either as path or as html, not both and not neither",
            ))
        }
    };
    if page.len() as u64 > MAX_PAGE_BYTES {
        return Err(refusal(
            "the page is larger than 1 MB; trim its data or markup",
        ));
    }
    if page.trim().is_empty() {
        return Err(refusal("the page is empty"));
    }
    Ok(page)
}

impl KernelRuntimeState {
    fn visual_run(&self, token: &str) -> Result<(String, Option<std::path::PathBuf>), DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(refusal("visual tools need one authenticated provider run"));
        };
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Err(refusal("the provider run has ended"));
        }
        Ok((
            run.session_id().to_string(),
            run.working_directory().cloned(),
        ))
    }

    pub(crate) async fn visual_tool(
        &self,
        token: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session_id, directory) = self.visual_run(token)?;
        let invalid = |error: serde_json::Error| refusal(format!("invalid arguments: {error}"));
        match canonical(tool) {
            Some(VISUAL_SHOW) => {
                let args: ShowArgs = serde_json::from_value(arguments).map_err(invalid)?;
                let page = page_text(args.path, args.html, directory.as_deref())?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: store_page(&session_id, &page, args.wide)?,
                })
            }
            Some(VISUAL_PREVIEW) => {
                let args: PreviewArgs = serde_json::from_value(arguments).map_err(invalid)?;
                let page = page_text(args.path, args.html, directory.as_deref())?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: super::visual_preview::preview(&page, args.width.unwrap_or(900))
                        .await?,
                })
            }
            _ => Err(refusal("unknown visual tool")),
        }
    }
}

/// Keeps the page under the kernel's state, named by its content: showing the same page twice is one file, and the copy
/// outlives the file it came from. The client reads it back by path.
fn store_page(session_id: &str, page: &str, wide: bool) -> Result<serde_json::Value, DaemonError> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(refusal("the Room id cannot name a folder"));
    }
    let id = format!("{:x}", Sha256::digest(page.as_bytes()))[..24].to_string();
    let directory = crate::config::default_state_dir()
        .join("visuals")
        .join(session_id);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|error| refusal(format!("cannot keep the page: {error}")))?;
    let path = directory.join(format!("{id}.html"));
    if !path.exists() {
        let partial = directory.join(format!("{id}.partial"));
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&partial)
            .and_then(|mut file| std::io::Write::write_all(&mut file, page.as_bytes()))
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|error| refusal(format!("cannot keep the page: {error}")))?;
    }
    Ok(serde_json::json!({
        "visual_id": id,
        "path": path.display().to_string(),
        "bytes": page.len(),
        "wide": wide,
        "note": "Shown to the reader above your reply. Don't mention or describe the page."
    }))
}
