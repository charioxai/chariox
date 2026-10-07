# Inter-Provider Context Switching

## Goal

When a user changes an agent's provider, model, or account, the agent keeps its conversation. The kernel owns that continuity and survives kernel restarts. Its one model call is the handoff brief, written through the target's official harness, and the switch never depends on it.

## Policy

A model or effort change within the same provider and account keeps the provider's native session: Codex resumes the thread with `thread/resume` and the new `model`, Claude launches with `--resume <session> --model <model>`, and OpenCode resumes its session. The provider keeps the whole conversation and compacts it for the new model's window itself. `ProviderResumeState::after_profile_change` is the single rule for local, home-remote, and worker-leased profile updates.

A provider or account change cannot reuse the native session, which lives in the other provider's or account's store, so the kernel transfers the conversation. The handoff is derived from operational history at dispatch: a prompt carries it when the run's native session has never answered this agent. The operational history store records each provider session that answered an agent, so history retention cannot erase it. The kernel keeps no handoff state of its own. The rule therefore applies equally when the switch happened without a live run, across a kernel restart, or after a provider could not resume its session. Run ids restart with the kernel, so an answer is a run's own only when it was given since that run started. A conversation that no provider ever answered still transfers its prompts and errors.

The handoff goes out once per new session. After the run receives it, later prompts to that run skip it, including queued prompts and the next prompt after a failed first turn. Steering prompts join the turn that already carried it. A source answer recorded without a provider session ID still transfers. Workflow fresh-context runs and turn substitutes never receive it this way.

Fork targets and turn substitutes start outside the agent's own conversation and keep an explicit one-shot handoff, consumed when the provider accepts the turn.

Stored user history, prompt queues, and terminal echoes remain the original user text.

## Handoff contents

The packet's byte budget is strict: 15% of the target model's context window at 3 bytes per token, within the 256 KB prompt transport. That is 90 KB for a 200k Claude model and 116 KB for Codex's 258,400-token window. Claude native turns take whatever room the turn's other hidden context leaves under the 48 KB hook transport, so a switch never fails the turn. The Claude and Codex catalogs report no window, so `provider::model_context_window_tokens` holds each harness's documented one.

The packet holds, in order:

1. The handoff brief, when the agent has one.
2. Facts the kernel recorded, not a model:
   - the worktree and branch;
   - the files modified and read, from tool calls and detected commits;
   - the commands run with their exit status;
   - open interactions;
   - prompts queued behind the current request.
3. Every prior user prompt, newest first, because prompts carry the requests, facts, and decisions.
4. Prior assistant answers, newest first. The three turns before the latest keep up to 3 KB each, as the verbatim recent tail.
5. The latest completed turn: its user prompt, its answer, and one line per tool call with its outcome, plus its status and error details. A retried request instead carries its interrupted attempt's output and provider details here, with its duplicated user line removed. Kernel notices alone do not make a dispatching request an interrupted attempt. The prompt being dispatched is the request itself and is never part of the packet.
6. Turns that do not fit are named with a pointer to the `chariox.search_recall` tool.

Under a tight budget, prior turns give way first, then the facts, the latest turn and the brief.

## Handoff brief

The brief is a structured summary with these sections:

- Goal
- Constraints & Preferences
- Progress: Done, In Progress, Blocked
- Key Decisions
- Next Steps
- Critical Context

The kernel stores it per agent in operational history, with the history sequence it covers.

How it is written:

- **When.** Before a provider-switch handoff is dispatched to a Codex or Chariox Claude run, the kernel folds the history after the watermark into the stored brief. It never runs for an intra-provider switch, which keeps the native session.
- **Engine.** One utility call on the target run's official harness. It runs in a fresh, metadata-only session: no tools, no MCP, no prior thread, and a private empty working directory.
- **Update rules.** The call applies PRESERVE / ADD / UPDATE rules to the previous brief.
- **Long histories.** History is read oldest first, in transcript chunks of up to 160 KB. Tool calls are reduced to their command, outcome and a short output excerpt; the full output stays in history for recall.
- **Progress.** Each chunk's result is stored at once, so later switches fold in only the new turns.

Which model writes it:

- `history.handoff.codex_brief_model` or `history.handoff.claude_brief_model`, when set.
- Otherwise, on Codex, `gpt-6-luna`: it writes an L brief in about 5 s, where gpt-5.5 took up to 19 s.
- Otherwise the source model, when the target harness can run it, as on an account switch.
- Otherwise the target model.
- If the chosen model fails on the first call, for instance because the account cannot run it, the brief is retried once on the target model.

The update has 120 seconds. If it fails, times out, or the target is a Claude native run, the packet keeps the stored brief, if any, and its deterministic sections.

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
- `runtime::state::context_handoff::builder::tests::the_packet_leads_with_the_brief_and_the_kernel_facts`
- `runtime::state::context_handoff::facts::tests::facts_come_from_codex_and_claude_tool_calls_and_commits`
- `runtime::state::context_handoff::brief::tests::long_history_is_read_oldest_first_in_bounded_chunks`
- `runtime::state::context_handoff::tests::a_switch_carries_the_stored_brief_and_the_packet_fits_the_target_window`
- `history::handoff_brief::tests::a_stored_brief_never_moves_its_watermark_back`
- Live drill: `apps/cli/scripts/live-model-switch-context-drill.mjs`. It plants facts and can add filler turns or restart the kernel before or after the switch. It then changes the agent profile and probes recall without tools. For long sessions:
  - `--bulk-turns N --bulk-kb K` scripts a session that outgrows the target window;
  - `--rich-probes` adds an answer-only fact, a superseded decision, the current task and next step, a file a tool created, and the latest tool result;
  - `--allow-recall` lets the recall turn use `chariox.search_recall`.
