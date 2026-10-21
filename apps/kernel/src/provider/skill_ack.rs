//! MP-08 / MP-10 / MP-11: receipts from official completed runtime-tool results.
//! This observes provider protocol data before transcript truncation. It never
//! accepts assistant prose, a tool invocation alone, or a failed response.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillToolDeliveryAcknowledgement {
    pub prompt_id: String,
    pub skill_id: String,
    pub digest: String,
}

pub(crate) fn completed_skill_tool(
    tool: &str,
    arguments: &Value,
    output: &Value,
) -> Option<SkillToolDeliveryAcknowledgement> {
    if crate::transport::runtime_tools::canonical_skill_tool_name(tool)
        != Some(crate::transport::runtime_tools::LOAD_SKILL_TOOL)
    {
        return None;
    }
    let args: crate::transport::runtime_tools::LoadSkillArgs =
        serde_json::from_value(arguments.clone()).ok()?;
    let origin = args.origin_prompt_id.as_str();
    let skill_id = args.skill_id.as_str();
    if origin.is_empty() || origin.len() > 256 || skill_id.is_empty() || skill_id.len() > 128 {
        return None;
    }
    let digest = delivered_digest(output, skill_id, 0)?;
    Some(SkillToolDeliveryAcknowledgement {
        prompt_id: origin.into(),
        skill_id: skill_id.into(),
        digest,
    })
}

/// Normalized official Codex/OpenCode tool frames retain full result bytes.
/// Claude observes the raw matching result before its bounded transcript copy.
pub(crate) fn completed_tool_transcript(
    provider: &str,
    kind: crate::terminal::TerminalOutputKind,
    bytes: &[u8],
) -> Option<SkillToolDeliveryAcknowledgement> {
    if kind != crate::terminal::TerminalOutputKind::ProviderTool {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("status").and_then(Value::as_str) != Some("completed")
        || value
            .get("error")
            .is_some_and(|error| !error.is_null() && error.as_str() != Some(""))
    {
        return None;
    }
    let tool = value.get("tool")?.as_str()?;
    if provider == "codex" {
        if value.get("title").and_then(Value::as_str) != Some("chariox") {
            return None;
        }
    } else if !tool.starts_with("chariox.")
        && !tool.starts_with("chariox_")
        && !tool.starts_with("mcp__chariox__")
    {
        return None;
    }
    completed_skill_tool(tool, value.get("input")?, value.get("output")?)
}

fn delivered_digest(value: &Value, skill_id: &str, depth: u8) -> Option<String> {
    if depth > 4 || value.get("isError").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if value.get("kind").and_then(Value::as_str) == Some("loaded")
        && value.get("skill_id").and_then(Value::as_str) == Some(skill_id)
        && value
            .get("body")
            .and_then(Value::as_str)
            .is_some_and(|body| !body.is_empty())
    {
        let digest = value.get("digest")?.as_str()?;
        return (digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| digest.into());
    }
    if let Some(content) = value
        .get("structuredContent")
        .filter(|content| !content.is_null())
    {
        return delivered_digest(content, skill_id, depth + 1);
    }
    if let Some(text) = value.as_str() {
        let content: Value = serde_json::from_str(text).ok()?;
        return delivered_digest(&content, skill_id, depth + 1);
    }
    let blocks = value
        .as_array()
        .or_else(|| value.get("content")?.as_array())?;
    // The ordinary MCP response contains one JSON text block. Do not treat
    // arbitrary later text as a successful replacement for an error/other result.
    let text = blocks
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))?
        .get("text")?
        .as_str()?;
    delivered_digest(&Value::String(text.into()), skill_id, depth + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn body() -> Value {
        json!({"kind":"loaded","skill_id":"skill-1","digest":"a".repeat(64),"body":"exact reviewed instructions","root":"/owned/skill-1/digest","files":[]})
    }
    #[test]
    fn skill_tool_ack_reads_official_mcp_and_claude_results_before_display_truncation() {
        let args = json!({"origin_prompt_id":"turn-1","skill_id":"skill-1"});
        for output in [
            body(),
            Value::String(body().to_string()),
            json!({"structuredContent":body(),"isError":false}),
            json!({"content":[{"type":"text","text":body().to_string()}]}),
            // Real Codex MCP results include an explicit null structuredContent.
            json!({"_meta":null,"structuredContent":null,"content":[{"type":"text","text":body().to_string()}]}),
            json!([{"type":"text","text":body().to_string()}]),
        ] {
            let receipt = completed_skill_tool("mcp__chariox__load_skill", &args, &output).unwrap();
            assert_eq!(receipt.prompt_id, "turn-1");
            assert_eq!(receipt.skill_id, "skill-1");
            assert_eq!(receipt.digest, "a".repeat(64));
        }
    }
    #[test]
    fn skill_tool_ack_refuses_invocation_failure_wrong_identity_and_forged_explicit() {
        let args = json!({"origin_prompt_id":"turn-1","skill_id":"skill-1"});
        for output in [
            json!({"kind":"page","skills":[]}),
            json!({"isError":true,"structuredContent":body()}),
            Value::Null,
            json!({"content":[{"type":"text","text":"tool failed"},{"type":"text","text":body().to_string()}]}),
        ] {
            assert!(completed_skill_tool("load_skill", &args, &output).is_none());
        }
        assert!(completed_skill_tool("other.load_skill", &args, &body()).is_none());
        assert!(completed_skill_tool(
            "load_skill",
            &json!({"skill_id":"other","origin_prompt_id":"turn-1"}),
            &body()
        )
        .is_none());
        assert!(
            completed_skill_tool("load_skill", &json!({"skill_id":"skill-1"}), &body()).is_none()
        );
        assert!(completed_skill_tool(
            "load_skill",
            &json!({"skill_id":"skill-1","origin_prompt_id":"turn-1","explicit":true}),
            &body()
        )
        .is_none());
    }
    #[test]
    fn skill_tool_transcript_ack_rejects_prose_failure_and_foreign_mcp_servers() {
        use crate::terminal::TerminalOutputKind;
        let mut value = json!({"id":"call-1","tool":"chariox.load_skill","title":"chariox","status":"completed","input":{"origin_prompt_id":"turn-1","skill_id":"skill-1"},"output":body().to_string()});
        assert!(completed_tool_transcript(
            "codex",
            TerminalOutputKind::ProviderTool,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_some());
        assert!(completed_tool_transcript(
            "opencode",
            TerminalOutputKind::ProviderTool,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_some());
        assert!(completed_tool_transcript(
            "codex",
            TerminalOutputKind::ProviderOutput,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_none());
        value["title"] = json!("foreign");
        assert!(completed_tool_transcript(
            "codex",
            TerminalOutputKind::ProviderTool,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_none());
        value["title"] = json!("chariox");
        value["status"] = json!("running");
        assert!(completed_tool_transcript(
            "codex",
            TerminalOutputKind::ProviderTool,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_none());
        value["status"] = json!("completed");
        value["error"] = json!("denied");
        assert!(completed_tool_transcript(
            "codex",
            TerminalOutputKind::ProviderTool,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_none());
    }
}
