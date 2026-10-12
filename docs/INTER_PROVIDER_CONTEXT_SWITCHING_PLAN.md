# Inter-Provider Context Switching

## Goal

When a user changes an agent from one provider/model to another, Chariox should carry enough recent context into the new provider session for the next provider-bound prompt to make sense. The handoff must be deterministic, bounded, and must not call an LLM unless the user explicitly asks for the provider switch and we later add an opt-in summary path.

## Policy

The kernel owns the handoff. On a successful provider launch, if the previous active run belonged to the same agent and the provider or model changed, the kernel builds a one-shot context packet from operational history and stores it in memory for that session/agent.

The packet is injected only into the next prompt sent to that agent, including workflow prompts. Stored user history, prompt queues, and terminal echoes remain the original user text.

The context packet is bounded and prioritized:

1. Current/latest turn gets detail: latest user prompt, assistant output/status/error, and tool details from that turn.
2. Older turns get only user prompts plus assistant output summaries/snippets.
3. Stable facts and status/error details are included only for the latest turn.
4. Tool use is included only for the latest turn. Older tool output is excluded.

MP-08 / MP-10 / MP-11: when a worker restart recreates its lease, the home reconstructs its protected conversation into the existing hidden prompt context before the first worker submission. Every existing lease refresh path records a private, durable marker for that exact execution lease before publishing the new binding; the first accepted replay-bearing submission consumes it. The marker retains the exact replay-bearing prompt identity so receipt reconciliation applies the same rule after a home restart. Continuing workflow turns replay; only the ordinary workflow fresh-context policy skips replay, leaving the marker for a later ordinary turn. Ordinary follow-up turns and agent moves do not trigger the recreated-lease packet or a full history read. Rejected submissions retain the marker for retry, including across a home restart. The user prompt and home history remain unchanged; fresh workflow turns keep their separate context contract.

MP-08 / MP-10 / MP-11: Claude sizes the optional replay after adding worker instructions. It removes older summaries before latest-turn detail at UTF-8 boundaries and labels omissions with `[truncated]`. Mandatory instructions remain intact. If no replay payload fits, submission rejects instead of silently consuming the conversation.

MP-08 / MP-10 / MP-11: unavailable Room protection withholds all cached background and history rather than failing the active user prompt. Required worker instructions are assembled normally. No replay is acknowledged while it is withheld, so the marker remains pending until protection recovers.

## Phases

### Phase 1: Deterministic Handoff Builder

Add a pure builder over operational history events. It partitions events at the last `user_prompt`, formats prior turns compactly, formats the latest turn with detail, and enforces a fixed character budget.

### Phase 2: Provider Switch Capture

On provider launch success, detect same-agent provider/model switches. Load the agent's operational history, build the handoff packet, and save it in a pending in-memory store keyed by session and agent.

### Phase 3: One-Shot Prompt Injection

At prompt dispatch time, after liveness checks and before provider input is written, consume any pending handoff for the session/agent and prepend it to the provider prompt. This applies to user prompts and workflow prompts because both need provider-session continuity after a switch.

### Phase 4: Validation Drills

Run targeted tests and manual drills to verify:

1. Switching providers after prior work injects past user prompts and assistant snippets.
2. Latest-turn tool/error details are included.
3. Older tool output is not included.
4. The handoff is consumed once and not repeated on the second prompt.
5. Budget pressure drops older context before the latest turn.
6. Workflow prompts receive the handoff when they are the first prompt after a provider switch.

Implemented kernel drills:

- `runtime::state::context_handoff::tests::older_tool_output_is_excluded_but_latest_tool_output_is_included`
- `runtime::state::context_handoff::tests::handoff_is_bounded_under_large_history`
- `runtime::state::context_handoff::tests::pending_handoff_is_consumed_once`
