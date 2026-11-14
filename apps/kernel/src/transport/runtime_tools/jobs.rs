use super::RuntimeToolSpec;
pub(super) fn job_tool_specs() -> Vec<RuntimeToolSpec> {
    let common =
        serde_json::json!({"task_id":{"type":"string"},"origin_prompt_id":{"type":"string"}});
    [("run", "Run builds, test suites and heavy commands as kernel-owned jobs. The kernel chooses local or remote placement from the Room's Remote jobs option, applies resource admission, and wakes you durably with the result. env accepts project environment NAMES only. Outputs are extra workspace-relative globs to return, including ignored build artifacts. A job outlives this tool call; yield on registration_id when waiting.", serde_json::json!({"command":{"oneOf":[{"type":"string","minLength":1},{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64}]},"cwd":{"type":"string"},"env":{"type":"array","items":{"type":"string"}},"needs":{"type":"object","properties":{"memory_gib":{"type":"integer","minimum":1},"cpus":{"type":"integer","minimum":1}},"additionalProperties":false},"timeout_s":{"type":"integer","minimum":1,"maximum":86400,"default":3600},"wait_s":{"type":"integer","minimum":0,"maximum":600,"default":0},"outputs":{"type":"array","items":{"type":"string"}}}),vec!["command"]),
    ("status", "Read your job's placement, state and finished result, including bounded output and the log artifact.", serde_json::json!({"job_id":{"type":"string"}}),vec!["job_id"]),
    ("cancel", "Cancel your job's owned process tree and release its remote machine. The original completion source settles after the process exits.", serde_json::json!({"job_id":{"type":"string"}}),vec!["job_id"])]
    .into_iter().map(|(name,description,extra,required)| {
        let mut properties = common.as_object().unwrap().clone(); properties.extend(extra.as_object().unwrap().clone());
        let mut fields=vec!["task_id","origin_prompt_id"]; fields.extend(required);
        RuntimeToolSpec { name:format!("chariox.jobs.{name}"),description:description.into(),input_schema:serde_json::json!({"type":"object","properties":properties,"required":fields,"additionalProperties":false}) }
    }).collect()
}
