//! MP-08/MP-09/MP-10/MP-11: agent-only browser operations. They reach the host as parameters of the tool call;
//! the terminal protocol keeps its commands.
use super::kernel_browser_runtime::host_error;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    command: ToolCommand,
    #[serde(default)]
    document_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum ToolCommand {
    /// An element, a viewport region (CSS px) or the whole page instead of the viewport.
    Screenshot {
        tab_id: String,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clip: Option<Clip>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_page: Option<bool>,
    },
    /// The bytes of a resource the page shows, written to a file in the Room workspace.
    Save {
        tab_id: String,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        path: String,
    },
    /// Until the text, the node or the URL part shows, or `timeout_ms` (at most 30000) passes.
    Wait {
        tab_id: String,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    /// A scroll that names a snapshot node instead of a pointer position.
    Input {
        tab_id: String,
        generation: u64,
        input: NodeScroll,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum NodeScroll {
    Scroll { target: String },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Clip {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// True for the calls this module owns: the other operations keep the terminal command path.
pub(super) fn handles(arguments: &serde_json::Value) -> bool {
    let command = &arguments["command"];
    match command["op"].as_str() {
        Some("screenshot") => ["node_ref", "clip", "full_page"]
            .iter()
            .any(|key| command.get(key).is_some()),
        Some("save" | "wait") => true,
        Some("input") => {
            command["input"]["kind"] == "scroll" && command["input"].get("target").is_some()
        }
        _ => false,
    }
}

const NODE_REF_REASON: &str = "node_ref must be a top-level backend:N from a fresh snapshot";
const MAX_URL_BYTES: usize = 16 * 1024;

fn observed_tab(tab_id: &str, generation: u64) -> bool {
    !tab_id.is_empty() && tab_id.len() <= 256 && generation > 0
}

fn node_ref_is_top_level(node_ref: &str) -> bool {
    node_ref.strip_prefix("backend:").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 16
            && !id.starts_with('0')
            && id.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// The longest one `wait` call may last; the kernel joins host slices up to this.
pub(super) const WAIT_LIMIT_MS: u32 = 30_000;

pub(super) fn params(arguments: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
    let request: Arguments = serde_json::from_value(arguments)
        .map_err(|error| host_error(super::kernel_browser_command_error::public_reason(&error)))?;
    let invalid = |reason: &str| host_error(format!("MD-3: invalid browser command ({reason})"));
    match &request.command {
        ToolCommand::Screenshot {
            tab_id,
            generation,
            node_ref,
            clip,
            full_page,
        } => {
            if !observed_tab(tab_id, *generation) {
                return Err(invalid("screenshot needs tab_id and generation"));
            }
            let chosen = [node_ref.is_some(), clip.is_some(), full_page.is_some()];
            if chosen.into_iter().filter(|chosen| *chosen).count() != 1 {
                return Err(invalid(
                    "screenshot takes one of node_ref, clip or full_page",
                ));
            }
            if *full_page == Some(false) {
                return Err(invalid("full_page must be true, or omitted"));
            }
            if node_ref
                .as_deref()
                .is_some_and(|node_ref| !node_ref_is_top_level(node_ref))
            {
                return Err(invalid(NODE_REF_REASON));
            }
            if clip
                .as_ref()
                .is_some_and(|clip| clip.width == 0 || clip.height == 0)
            {
                return Err(invalid("clip width and height must be at least 1"));
            }
        }
        ToolCommand::Wait {
            tab_id,
            generation,
            text,
            node_ref,
            url_contains,
            timeout_ms,
        } => {
            if !observed_tab(tab_id, *generation) {
                return Err(invalid("wait needs tab_id and generation"));
            }
            let chosen = [text.is_some(), node_ref.is_some(), url_contains.is_some()];
            if chosen.into_iter().filter(|chosen| *chosen).count() != 1 {
                return Err(invalid("wait takes one of text, node_ref or url_contains"));
            }
            if node_ref
                .as_deref()
                .is_some_and(|node_ref| !node_ref_is_top_level(node_ref))
            {
                return Err(invalid(NODE_REF_REASON));
            }
            if [text, url_contains]
                .into_iter()
                .flatten()
                .any(|needle| needle.is_empty() || needle.chars().count() > 512)
            {
                return Err(invalid(
                    "wait text and url_contains must be 1 through 512 characters",
                ));
            }
            if timeout_ms.is_some_and(|timeout| timeout > WAIT_LIMIT_MS) {
                return Err(invalid("wait timeout_ms is at most 30000"));
            }
        }
        ToolCommand::Input {
            tab_id,
            generation,
            input: NodeScroll::Scroll { target },
        } => {
            if !observed_tab(tab_id, *generation) {
                return Err(invalid("input needs tab_id and generation"));
            }
            if !node_ref_is_top_level(target) {
                return Err(invalid(NODE_REF_REASON));
            }
        }
        ToolCommand::Save {
            tab_id,
            generation,
            node_ref,
            url,
            path,
        } => {
            if !observed_tab(tab_id, *generation) {
                return Err(invalid("save needs tab_id and generation"));
            }
            if node_ref.is_some() == url.is_some() {
                return Err(invalid("save takes a node_ref or a url, not both"));
            }
            if node_ref
                .as_deref()
                .is_some_and(|node_ref| !node_ref_is_top_level(node_ref))
            {
                return Err(invalid(NODE_REF_REASON));
            }
            if url
                .as_deref()
                .is_some_and(|url| url.is_empty() || url.len() > MAX_URL_BYTES)
            {
                return Err(invalid("save url must be 1 through 16384 bytes"));
            }
            if path.is_empty() || path.len() > 4096 || path.contains('\0') {
                return Err(invalid("save needs a path of 1 through 4096 bytes"));
            }
        }
    }
    let mut params = serde_json::to_value(&request.command).map_err(|_| invalid("unsupported"))?;
    if matches!(request.command, ToolCommand::Input { .. }) {
        match request.document_id {
            Some(id) if !id.is_empty() && id.len() <= 256 => params["document_id"] = id.into(),
            _ => return Err(invalid("input needs the top-level document_id of the latest tab, snapshot or screenshot result")),
        }
    }
    params["focused_agent"] = true.into();
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_plain_screenshot_keeps_the_terminal_command_path() {
        assert!(!handles(
            &json!({"command":{"op":"screenshot","tab_id":"t","generation":1}})
        ));
        assert!(!handles(&json!({"command":{"op":"state"}})));
    }

    #[test]
    fn a_region_screenshot_becomes_host_parameters() {
        let clip = json!({"command":{"op":"screenshot","tab_id":"t","generation":2,"clip":{"x":1,"y":2,"width":30,"height":40}}});
        assert!(handles(&clip));
        let sent = params(clip).unwrap();
        assert_eq!(sent["op"], "screenshot");
        assert_eq!(sent["clip"], json!({"x":1,"y":2,"width":30,"height":40}));
        assert_eq!(sent["focused_agent"], true);
        assert!(sent.get("node_ref").is_none() && sent.get("full_page").is_none());
        let node = params(json!({"command":{"op":"screenshot","tab_id":"t","generation":2,"node_ref":"backend:7"}})).unwrap();
        assert_eq!(node["node_ref"], "backend:7");
        let page = params(
            json!({"command":{"op":"screenshot","tab_id":"t","generation":2,"full_page":true}}),
        )
        .unwrap();
        assert_eq!(page["full_page"], true);
    }

    #[test]
    fn a_scroll_to_a_node_is_an_input_bound_to_an_observed_document() {
        let scroll = json!({"document_id":"doc","command":{"op":"input","tab_id":"t","generation":1,"input":{"kind":"scroll","target":"backend:7"}}});
        assert!(handles(&scroll));
        let sent = params(scroll.clone()).unwrap();
        assert_eq!(sent["input"], json!({"kind":"scroll","target":"backend:7"}));
        assert_eq!(sent["document_id"], "doc");
        assert_eq!(sent["focused_agent"], true);
        let mut without = scroll.clone();
        without.as_object_mut().unwrap().remove("document_id");
        assert!(super::params(without).is_err());
        for input in [
            json!({"kind":"scroll","target":"backend:7","x":1,"y":2,"delta_x":0,"delta_y":5}),
            json!({"kind":"scroll","target":"frame:a:backend:7"}),
            json!({"kind":"click","target":"backend:7"}),
        ] {
            let mut bad = scroll.clone();
            bad["command"]["input"] = input;
            assert!(
                !handles(&bad) || super::params(bad.clone()).is_err(),
                "{bad}"
            );
        }
        assert!(!handles(
            &json!({"document_id":"doc","command":{"op":"input","tab_id":"t","generation":1,"input":{"kind":"scroll","x":1,"y":2,"delta_x":0,"delta_y":5}}})
        ));
    }

    #[test]
    fn a_wait_names_one_condition_and_a_bounded_timeout() {
        let wait = json!({"command":{"op":"wait","tab_id":"t","generation":1,"text":"Done","timeout_ms":5000}});
        assert!(handles(&wait));
        let sent = params(wait).unwrap();
        assert_eq!(
            (sent["text"].as_str(), sent["timeout_ms"].as_u64()),
            (Some("Done"), Some(5000))
        );
        assert_eq!(sent["focused_agent"], true);
        for command in [
            json!({"op":"wait","tab_id":"t","generation":1}),
            json!({"op":"wait","tab_id":"t","generation":1,"text":"a","url_contains":"b"}),
            json!({"op":"wait","tab_id":"t","generation":1,"text":""}),
            json!({"op":"wait","tab_id":"t","generation":1,"text":"a","timeout_ms":30001}),
            json!({"op":"wait","tab_id":"t","generation":1,"text":"a","timeout_ms":-1}),
            json!({"op":"wait","tab_id":"t","generation":1,"node_ref":"7"}),
            json!({"op":"wait","tab_id":"t","generation":1,"text":"a","_agent_input":true}),
        ] {
            assert!(
                super::params(json!({ "command": command })).is_err(),
                "accepted {command}"
            );
        }
    }

    #[test]
    fn a_save_becomes_host_parameters_without_caller_roots() {
        let save = json!({"command":{"op":"save","tab_id":"t","generation":2,"node_ref":"backend:7","path":"images/logo.png"}});
        assert!(handles(&save));
        let sent = params(save).unwrap();
        assert_eq!(sent["op"], "save");
        assert_eq!(sent["path"], "images/logo.png");
        assert_eq!(sent["node_ref"], "backend:7");
        assert_eq!(sent["focused_agent"], true);
        let by_url = params(json!({"command":{"op":"save","tab_id":"t","generation":2,"url":"https://a.example/x.png","path":"x.png"}})).unwrap();
        assert!(by_url.get("node_ref").is_none());
        for extra in ["_save_roots", "_upload_roots", "_agent_input"] {
            let mut command = json!({"op":"save","tab_id":"t","generation":2,"url":"https://a.example/x.png","path":"x.png"});
            command[extra] = json!(["/"]);
            assert!(
                params(json!({ "command": command })).is_err(),
                "{extra} is the kernel's"
            );
        }
    }

    #[test]
    fn a_save_without_exactly_one_source_or_with_a_bad_path_is_refused() {
        for command in [
            json!({"op":"save","tab_id":"t","generation":1,"path":"x.png"}),
            json!({"op":"save","tab_id":"t","generation":1,"url":"https://a.example/x","node_ref":"backend:7","path":"x.png"}),
            json!({"op":"save","tab_id":"t","generation":1,"url":"https://a.example/x"}),
            json!({"op":"save","tab_id":"t","generation":1,"url":"https://a.example/x","path":""}),
            json!({"op":"save","tab_id":"t","generation":1,"url":"https://a.example/x","path":"a\u{0}b"}),
            json!({"op":"save","tab_id":"t","generation":1,"node_ref":"frame:a:backend:7","path":"x.png"}),
            json!({"op":"save","tab_id":"t","generation":0,"url":"https://a.example/x","path":"x.png"}),
        ] {
            assert!(
                params(json!({ "command": command })).is_err(),
                "accepted {command}"
            );
        }
    }

    #[test]
    fn a_region_screenshot_with_several_regions_or_bad_values_is_refused() {
        for command in [
            json!({"op":"screenshot","tab_id":"t","generation":1,"node_ref":"backend:7","full_page":true}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"full_page":false}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"node_ref":"frame:a:backend:7"}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"node_ref":"backend:0"}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"clip":{"x":0,"y":0,"width":0,"height":5}}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"clip":{"x":0,"y":0,"width":5}}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"clip":{"x":-1,"y":0,"width":5,"height":5}}),
            json!({"op":"screenshot","tab_id":"t","generation":0,"full_page":true}),
            json!({"op":"screenshot","tab_id":"t","generation":1,"full_page":true,"_capture_protection":true}),
        ] {
            assert!(
                params(json!({ "command": command })).is_err(),
                "accepted {command}"
            );
        }
    }
}
