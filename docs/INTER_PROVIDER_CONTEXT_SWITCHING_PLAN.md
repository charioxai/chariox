# Inter-Provider Context Switching

## Goal

When a user changes an agent's provider, model, or account, the agent keeps its conversation. The kernel owns that continuity, never calls an LLM for it, and survives kernel restarts.

## Policy

A model or effort change within the same provider and account keeps the provider's native session: Codex resumes the thread with `thread/resume` and the new `model`, Claude launches with `--resume <session> --model <model>`, and OpenCode resumes its session. The provider keeps the whole conversation and compacts it for the new model's window itself. `ProviderResumeState::after_profile_change` is the single rule for local, home-remote, and worker-leased profile updates.

MP-08 / MP-10 / MP-11: Claude's `<synthetic>` error-message model is not an execution model. A quota failure remains visible without replacing the selected model or preventing the next turn from reaching normal provider admission.

MP-08 / MP-10: a provider-only change selects the destination family’s registered default account and clears an omitted model or effort. A same-family change keeps the pinned account and existing selections; explicit destination selections still undergo account validation. This kernel rule applies to local and leased agents. MP-08 / MP-10: a delayed TUI account catalog keeps later model/effort selections, and cannot commit after a newer provider or account selection supersedes it.

A provider or account change cannot reuse the native session, which lives in the other provider's or account's store, so the kernel transfers the conversation. The handoff is derived from operational history at dispatch: a prompt carries it when the run's native session does not hold the latest conversation. Returning to a native session keeps that session and transfers the work done meanwhile on another provider or account. A historical answer alone does not establish that the session is current. The operational history store records each provider session that answered an agent, so history retention cannot erase that marker; it is a fallback only when the latest output lacks a native session ID and comes from the same provider family. The kernel keeps no durable handoff state of its own. The rule therefore applies equally when the switch happened without a live run, across a kernel restart, or after a provider could not resume its session. Run ids restart with the kernel, so an answer is a run's own only when it was given since that run started. A conversation that no provider ever answered still transfers its prompts and errors.

MP-08 / MP-10: the handoff goes out once per native session that needs the conversation, including a returning native session with intervening work. A native `/clear` that changes the session ID within the same kernel run receives the retained conversation again. After that native session receives it, later prompts to it skip it, including queued prompts and the next prompt after a failed first turn. Steering prompts join the turn that already carried it. A source answer recorded without a provider session ID still transfers. Workflow fresh-context runs and turn substitutes never receive it this way.

MP-08 / MP-10: the dispatcher prepares a native prompt’s hidden context before its first PTY injection. The output pump retries an existing native injection; it does not start one without the dispatcher’s handoff or replace a native user draft.

Fork targets and turn substitutes start outside the agent's own conversation and keep an explicit one-shot handoff, consumed when the provider accepts the turn.

MP-08 / MP-10: ready leased worker turns use the shared runtime dispatcher, so provider and account changes carry the conversation before submission. The worker admits the turn under the app lock and dispatches after releasing it.

Stored user history, prompt queues, and terminal echoes remain the original user text.

## Handoff contents

The packet is deterministic, and its byte budget is strict:

- Codex and Claude `-p` turns allow up to 24 KB.
- Claude native turns take whatever room the turn's other hidden context leaves under the 48 KB hook transport, capped at 24 KB. A switch therefore never fails the turn.

The packet fills its budget in this order, and the earlier items give way last:

1. The latest completed turn in detail: user prompt, assistant output, and its tool, status, and error details. A retried request instead carries its interrupted attempt's output and provider details here, with its duplicated user line removed. Kernel notices alone do not make a dispatching request an interrupted attempt. The prompt being dispatched is the request itself and is never part of the packet.
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
- `runtime::state::context_handoff::builder::tests::the_dispatching_prompt_leaves_the_last_completed_turn_latest`
- `runtime::state::context_handoff::builder::tests::claude_text_blocks_separated_by_thinking_stay_apart`
- `runtime::state::agent_config_runtime_state::tests::automatic_substitutes::the_substitute_and_the_next_primary_turn_see_the_conversation` (tool row, provider error and a single request)
- Live drill: `apps/cli/scripts/live-model-switch-context-drill.mjs` plants facts, optionally adds filler turns or restarts the kernel before or after the switch, changes the agent profile, and probes recall without tools.

MP-08 / MP-10 / MP-11: when a worker restart recreates its lease, the home reconstructs its protected conversation into the existing hidden prompt context before the first worker submission. Every existing lease refresh path records a private, durable marker for that exact execution lease before publishing the new binding; the first accepted replay-bearing submission consumes it. The marker retains the exact replay-bearing prompt identity so receipt reconciliation applies the same rule after a home restart. Continuing workflow turns replay; only the ordinary workflow fresh-context policy skips replay, leaving the marker for a later ordinary turn. Ordinary follow-up turns and agent moves do not trigger the recreated-lease packet or a full history read. Rejected submissions retain the marker for retry, including across a home restart. The user prompt and home history remain unchanged; fresh workflow turns keep their separate context contract.

MP-08 / MP-10 / MP-11: Claude sizes the optional replay after adding worker instructions. It removes older summaries before latest-turn detail at UTF-8 boundaries and labels omissions with `[truncated]`. Mandatory instructions remain intact. If no replay payload fits, submission rejects instead of silently consuming the conversation.

MP-08 / MP-10 / MP-11: unavailable Room protection withholds optional history replay rather than failing the active prompt. Current hidden context is sent unchanged, preserving workflow node instructions, delivery/acknowledgement contracts, failed-request notes, and steer context. No replay is acknowledged while it is withheld, so the marker remains pending until protection recovers.
