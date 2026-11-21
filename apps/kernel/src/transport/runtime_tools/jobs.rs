use super::RuntimeToolSpec;
pub(super) fn job_tool_specs() -> Vec<RuntimeToolSpec> {
    let common =
        serde_json::json!({"task_id":{"type":"string"},"origin_prompt_id":{"type":"string"}});
    [("run", "Run a command you expect to take minutes (a full build or test suite) as a kernel-owned job; run quick commands in your own shell. The kernel chooses local or remote placement from the Room's Remote jobs option, applies resource admission, and wakes you durably with the result. env accepts project environment NAMES only. Outputs are extra workspace-relative globs to return, including ignored build artifacts. A job outlives this tool call; yield on registration_id when waiting. chariox.jobs.status reads a saved result and chariox.jobs.cancel ends your job. The owner must approve remote work unless they enabled unattended machine acquisition. To run the job on a machine you already hold, pass machine: the allocation_ref chariox.machines.acquire returned; the job runs on that machine, uploads and returns files as any remote job does, never allocates or releases a machine, and asks the owner nothing more. For another machine enrolled under the same owner, pass machine as an object with kernel (its ID or name). With directory (an absolute folder on that machine), the job runs there and leaves its files there; use chariox.files.copy to transfer source and artifacts. Without directory, it runs in a staged copy of this workspace. The target needs no provider account. A machine that is not held, released or unreachable is refused with what to do. To acquire a machine for other work use chariox.machines.acquire, read chariox.machines.status until ready, and release it with chariox.machines.release when finished.", serde_json::json!({"command":{"oneOf":[{"type":"string","minLength":1},{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64}]},"cwd":{"type":"string"},"env":{"type":"array","items":{"type":"string"}},"needs":{"type":"object","properties":{"memory_gib":{"type":"integer","minimum":1},"cpus":{"type":"integer","minimum":1}},"additionalProperties":false},"timeout_s":{"type":"integer","minimum":1,"maximum":86400,"default":3600},"wait_s":{"type":"integer","minimum":0,"maximum":600,"default":0},"outputs":{"type":"array","items":{"type":"string"}},"machine":{"oneOf":[{"type":"string","minLength":1,"description":"Held allocation_ref"},{"type":"object","required":["kernel"],"properties":{"kernel":{"type":"string","minLength":1},"directory":{"type":"string","minLength":1}},"additionalProperties":false}]}}),vec!["command"]),
    ("list", "List only your own jobs in this Room, including unreadable retained rows and queued memory or disk pressure. No sudo is needed. Use status to open one result or cancel with a listed job_id to release your own work.", serde_json::json!({}), vec![]),
    ("status", "Read your job's placement, state and bounded output as it runs. When finished, read its result and log artifact.", serde_json::json!({"job_id":{"type":"string"}}),vec!["job_id"]),
    ("cancel", "Cancel your job's owned process tree and release any worker the job allocated. The original completion source settles after the process exits.", serde_json::json!({"job_id":{"type":"string"}}),vec!["job_id"])]
    .into_iter().map(|(name,description,extra,required)| {
        let mut properties = common.as_object().unwrap().clone(); properties.extend(extra.as_object().unwrap().clone());
        let mut fields=vec!["task_id","origin_prompt_id"]; fields.extend(required);
        RuntimeToolSpec { name:format!("chariox.jobs.{name}"),description:description.into(),input_schema:serde_json::json!({"type":"object","properties":properties,"required":fields,"additionalProperties":false}) }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::job_tool_specs;

    #[test]
    fn the_run_tool_says_quick_commands_stay_in_the_shell() {
        let specs = job_tool_specs();
        let run = specs
            .iter()
            .find(|spec| spec.name == "chariox.jobs.run")
            .expect("jobs.run spec");
        assert!(run.description.contains("expect to take minutes"));
        assert!(run.description.contains("quick commands in your own shell"));
        assert!(!run.description.contains("heavy commands"));
    }

    #[test]
    fn the_run_tool_offers_a_held_machine_by_its_allocation_ref() {
        let specs = job_tool_specs();
        let run = specs
            .iter()
            .find(|spec| spec.name == "chariox.jobs.run")
            .expect("jobs.run spec");
        assert_eq!(
            run.input_schema["properties"]["machine"]["oneOf"][0]["type"],
            "string"
        );
        assert!(run.description.contains("a machine you already hold"));
    }

    #[test]
    fn own_job_list_is_discoverable_with_only_authenticated_turn_arguments() {
        let specs = job_tool_specs();
        let list = specs
            .iter()
            .find(|spec| spec.name == "chariox.jobs.list")
            .unwrap();
        assert_eq!(
            list.input_schema["required"],
            serde_json::json!(["task_id", "origin_prompt_id"])
        );
        assert_eq!(
            super::super::canonical_agent_event_tool_name(&list.name),
            Some("chariox.jobs.list")
        );
        assert_eq!(
            super::super::canonical_agent_event_tool_name("mcp__chariox__chariox_jobs_list"),
            Some("chariox.jobs.list")
        );
    }
}
