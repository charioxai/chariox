use super::*;
use serde_json::{json, Value};

#[test]
fn prompt_control_receipts_fit_the_wire_budget() {
    let worktree = crate::test_support::TestWorktree::new("prompt-receipts");
    let harness = LocalRouterTestHarness::new();
    let budget = crate::transport::perf_budget::budget("prompt_control_receipt_bytes");
    let (session_id, agent_id) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => {
            (session.id().to_owned(), agent.id().to_owned())
        }
        other => panic!("expected a Room, got {other:?}"),
    };
    harness.launch_workflow_test_provider(&session_id, &agent_id);
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session_id.clone(),
                client_id: "receipt-test".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("expected an attachment, got {other:?}"),
    };
    let request = |kind: &str, payload: Value, expected: &str| {
        let result = harness
            .dispatch(serde_json::from_value(json!({ (kind): payload })).unwrap())
            .unwrap();
        let bytes = serde_json::to_vec(&result).unwrap();
        assert!(
            bytes.len() <= budget,
            "{kind} response took {} bytes, over its {budget} byte budget",
            bytes.len()
        );
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            response[expected].is_object(),
            "{kind} returned the wrong reply"
        );
        response[expected].clone()
    };

    let long_prompt = "long fixture content ".repeat(16_000);
    let attachments = json!([{
        "url": "https://example.invalid/prompt.txt",
        "mime": "text/plain",
        "filename": "prompt.txt"
    }]);
    let submit = |outcome: &str| {
        let reply = request(
            "SubmitPrompt",
            json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "target_agent_id": agent_id,
                "prompt": long_prompt,
                "attachments": attachments,
            }),
            "PromptSubmitted",
        );
        assert_eq!(reply["session_id"], session_id);
        assert!(reply.get("session").is_none());
        let receipt = &reply["outcome"][outcome]["prompt"];
        assert!(receipt.get("prompt").is_none() && receipt.get("attachments").is_none());
        receipt["id"]
            .as_str()
            .expect("accepted prompt ID")
            .to_owned()
    };
    let active = submit("Started");
    let steer = request(
        "SteerActivePrompt",
        json!({
            "session_id": session_id,
            "attachment_id": attachment.id(),
            "target_agent_id": agent_id,
            "expected_active_prompt_id": active,
            "prompt": long_prompt,
            "attachments": attachments,
        }),
        "ActivePromptSteered",
    );
    assert_eq!(steer["active_prompt_id"], active);
    assert!(steer["steer_id"].is_string());

    let queued = submit("Queued");
    let queue_fields = |prompt_id: &str| {
        json!({
            "session_id": session_id,
            "attachment_id": attachment.id(),
            "target_agent_id": agent_id,
            "prompt_id": prompt_id,
        })
    };
    let mut update = queue_fields(&queued);
    update["prompt"] = json!(long_prompt);
    for (kind, fields, tag) in [
        ("UpdateQueuedPrompt", update, "QueuedPromptUpdated"),
        (
            "CancelQueuedPrompt",
            queue_fields(&queued),
            "QueuedPromptCancelled",
        ),
    ] {
        let receipt = request(kind, fields, tag);
        assert_eq!(receipt["session_id"], session_id);
        assert_eq!(receipt["prompt"]["id"], queued);
        assert!(receipt.get("session").is_none());
    }
    let next = submit("Queued");
    let steered = request(
        "SteerQueuedPrompt",
        queue_fields(&next),
        "QueuedPromptSteered",
    );
    assert_eq!(steered["prompt"]["id"], next);
    let completed = request(
        "CompletePrompt",
        json!({"session_id": session_id}),
        "PromptCompleted",
    );
    assert_eq!(completed["completion"]["completed"]["id"], active);

    let batch = request(
        "SubmitPrompts",
        json!({
            "session_id": session_id,
            "attachment_id": attachment.id(),
            "prompts": [{"target_agent_id": agent_id, "prompt": long_prompt}],
        }),
        "PromptsSubmitted",
    );
    let batch_prompt = &batch["results"][0]["outcome"]["Started"]["prompt"];
    let batch_id = batch_prompt["id"].as_str().expect("accepted batch item ID");
    let cancelled = request(
        "CancelActivePrompt",
        json!({
            "session_id": session_id,
            "attachment_id": attachment.id(),
            "target_agent_id": agent_id,
            "expected_prompt_id": batch_id,
        }),
        "PromptCancelled",
    );
    assert_eq!(cancelled["cancellation"]["prompt"]["id"], batch_id);
}
