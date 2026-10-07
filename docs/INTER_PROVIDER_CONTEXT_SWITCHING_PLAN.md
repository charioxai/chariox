# Inter-Provider Context Switching

## Goal

When a user changes an agent's provider, model, or account, the agent keeps its conversation. The kernel owns that continuity, never calls an LLM for it, and survives kernel restarts.

## Policy

A model or effort change within the same provider and account keeps the provider's native session: Codex resumes the thread with `thread/resume` and the new `model`, Claude launches with `--resume <session> --model <model>`, and OpenCode resumes its session. The provider keeps the whole conversation and compacts it for the new model's window itself. `ProviderResumeState::after_profile_change` is the single rule for local, home-remote, and worker-leased profile updates.

A provider or account change cannot reuse the native session, which lives in the other provider's or account's store, so the kernel transfers the conversation. The handoff is derived from operational history at dispatch: a prompt carries it when the run's native session has never answered this agent. The operational history store records each provider session that answered an agent, so history retention cannot erase it. The kernel keeps no handoff state of its own. The rule therefore applies equally when the switch happened without a live run, across a kernel restart, or after a provider could not resume its session.

The handoff goes out once per new session. After the run receives it, later prompts to that run skip it, including queued prompts and the next prompt after a failed first turn. Steering prompts join the turn that already carried it. A source answer recorded without a provider session ID still transfers. Workflow fresh-context runs and turn substitutes never receive it this way.

Fork targets and turn substitutes start outside the agent's own conversation and keep an explicit one-shot handoff, consumed when the provider accepts the turn.

Stored user history, prompt queues, and terminal echoes remain the original user text.

## Handoff contents

The packet is deterministic, and its byte budget is strict:

- Codex and Claude `-p` turns allow up to 24 KB.
- Claude native turns take whatever room the turn's other hidden context leaves under the 48 KB hook transport, capped at 24 KB. A switch therefore never fails the turn.

The packet fills its budget in this order, and the earlier items give way last:

1. The latest turn in detail: user prompt, assistant output, and its tool, status, and error details.
2. Every prior user prompt, newest first, because prompts carry the requests, facts, and decisions.
3. Prior assistant answers, newest first, in the remaining budget.
4. Turns that do not fit are named with a pointer to the `chariox.search_recall` tool.

## Validation

- `runtime::state::context_handoff::tests::new_provider_session_receives_the_conversation_until_it_answers`
- `runtime::state::context_handoff::tests::a_new_session_receives_the_conversation_once`
- `runtime::state::context_handoff::tests::a_resumed_session_keeps_its_conversation_after_history_retention`
- `app::provider_output_claude_native::tests::claude_native_handoff_takes_only_the_room_its_turn_leaves`
- `runtime::state::context_handoff::builder::tests::handoff_shrinks_to_any_budget_and_keeps_the_latest_request_longest`
- `runtime::state::context_handoff::builder::tests::early_user_facts_survive_a_conversation_of_ordinary_turns`
- `runtime::state::context_handoff::builder::tests::handoff_is_bounded_under_large_history`
- `provider::launch_contract::tests::provider_resume_state_keeps_the_native_session_across_a_model_change`
- Live drill: `apps/cli/scripts/live-model-switch-context-drill.mjs` plants facts, optionally adds filler turns or restarts the kernel before or after the switch, changes the agent profile, and probes recall without tools.
