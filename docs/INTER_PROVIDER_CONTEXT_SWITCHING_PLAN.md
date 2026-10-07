# Inter-Provider Context Switching

## Goal

When a user changes an agent's provider, model, or account, the agent keeps its conversation. The kernel owns that continuity and survives kernel restarts. Its one model call is the handoff brief, written through the target's official harness, and the switch never depends on it.

## Policy

A model or effort change within the same provider and account keeps the provider's native session: Codex resumes the thread with `thread/resume` and the new `model`, Claude launches with `--resume <session> --model <model>`, and OpenCode resumes its session. The provider keeps the whole conversation and compacts it for the new model's window itself. `ProviderResumeState::after_profile_change` is the single rule for local, home-remote, and worker-leased profile updates.

Claude compacts only near the window of the model it runs. When a Claude model change keeps a live session on a smaller window, such as `sonnet` to `haiku`, and the session's last turn used more than 75% of the new window, the kernel first sends `/compact` to that session on its current model through the run's own stream-json runtime. A resolved Claude model keeps the selection's `[1m]`, so the run and the agent profile still name the 1M window after the first turn. The trigger uses the last assistant API call's input plus cached input, rather than the result's summed billing usage. The smaller model then resumes a summary it can hold, instead of summarizing a transcript larger than its window. The profile change waits for the compaction, up to 5 minutes. A failure leaves the session to Claude's own compaction. Codex compacts with the previous model by itself and needs no step.

A provider or account change cannot reuse the native session, which lives in the other provider's or account's store, so the kernel transfers the conversation. The handoff is derived from operational history at dispatch: a prompt carries it when the run's native session has never answered this agent. The operational history store records each provider session that answered an agent, so history retention cannot erase it. The kernel keeps no handoff state of its own. The rule therefore applies equally when the switch happened without a live run, across a kernel restart, or after a provider could not resume its session. Run ids restart with the kernel, so an answer is a run's own only when it was given since that run started. A conversation that no provider ever answered still transfers its prompts and errors.

The handoff goes out once per new session. After the run receives it, later prompts to that run skip it, including queued prompts and the next prompt after a failed first turn. Steering prompts join the turn that already carried it. A source answer recorded without a provider session ID still transfers. Workflow fresh-context runs and turn substitutes never receive it this way.

Fork targets and turn substitutes start outside the agent's own conversation and keep an explicit one-shot handoff, consumed when the provider accepts the turn.

Stored user history, prompt queues, and terminal echoes remain the original user text.

## Handoff contents

The packet's byte budget is strict: 15% of the target model's context window at 3 bytes per token, within the 256 KB prompt transport. That is 90 KB for a 200k Claude model and 116 KB for Codex's 258,400-token window. Current first-party Claude Code defaults are 1M for Sonnet 5/5.5, Opus 4.7/4.8/5/5.5 and Fable 5/5.1; older explicit model IDs and Haiku remain 200k unless `[1m]` is selected. Installed CLI 2.1.292 `/context` probes resolve plain `sonnet` and `opus` to 5.5 with 1M, `haiku` to 4.5 with 200k, and explicit Sonnet 4.6 to 200k. Alias pins and alternate deployments can differ. Claude native turns take whatever room the turn's other hidden context leaves under the 48 KB hook transport, so a switch never fails the turn. The Claude and Codex catalogs report no window, so `provider::model_context_window_tokens` holds each harness's documented one.

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

Every persisted brief read applies the Room's current observation-cache policy,
including utility input, direct handoffs and refresh-error fallback. Later secret
registration redacts cached text. Room recovery or a fence rejects the cache and
deletes its watermark; the next refresh rebuilds from protected history rather
than skipping older withheld events.

How it is written:

- **When.** Before a provider-switch handoff is dispatched to a Codex or Chariox Claude run, a detached dispatch continuation folds history after the watermark into the stored brief. The output pumps remain available, and session clients see “Preparing handoff brief…”. After preparation, the continuation takes the run's operation lane and checks prompt ownership and cancellation immediately before submission. Cancelled preparations never submit the request or save a completed fold. A deterministic packet that already preserves every turn needs no refresh. It never runs for an intra-provider switch, which keeps the native session.
- **Engine.** One utility call on the target run's official harness. It runs in a fresh, metadata-only session: no tools, no MCP, no prior thread, and a private empty working directory.
- **Update rules.** The call applies PRESERVE / ADD / UPDATE rules to the previous brief.
- **Long histories.** History is read oldest first, in transcript chunks of up to 160 KB. Tool calls are reduced to their command, outcome and a short output excerpt; the full output stays in history for recall.
- **Progress.** Each chunk's result is stored at once, so later switches fold in only the new turns.

Which model writes it:

- `history.handoff.codex_brief_model` or `history.handoff.claude_brief_model`, when set.
- Otherwise, on Codex, `gpt-6-luna`: it writes an L brief in about 5 s, where gpt-5.5 took up to 19 s.
- Otherwise the source model, when the target harness can run it, as on an account switch.
- Otherwise the target model.
- A definite unsupported-model rejection on the first call retries once on the target model. That rejection is remembered per account and exact model for the kernel lifetime; transient failures do not change models.

The update passes the remaining part of its 120-second deadline into each utility call, and waits for the utility to end before removing scratch. If it fails, times out, or the target is a Claude native run, the packet keeps the last successfully stored fold, if any, and its deterministic sections.

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
- `runtime::state::local_prompt_dispatch_runtime::tests::cancelling_during_handoff_brief_never_submits_or_saves_the_cancelled_fold`
- `runtime::state::local_prompt_dispatch_runtime::tests::claude_native_cancel_before_acknowledgement_settles_as_cancelled`
- `history::handoff_brief::tests::a_stored_brief_never_moves_its_watermark_back`
- `runtime::state::agent_profile_compaction::tests::only_a_session_too_large_for_the_smaller_claude_window_is_compacted_first`
- Live drill: `apps/cli/scripts/live-model-switch-context-drill.mjs`. It plants facts and can add filler turns or restart the kernel before or after the switch. It then changes the agent profile and probes recall without tools. For long sessions:
  - `--bulk-turns N --bulk-kb K` scripts a session that outgrows the target window;
  - `--rich-probes` adds an answer-only fact, a superseded decision, the current task and next step, a file a tool created, and the latest tool result;
  - `--allow-recall` lets the recall turn use `chariox.search_recall`.


Model downshifts within Claude reserve the idle agent through the kernel prompt owner. Owner, session, native-TUI and account validations run before `/compact`; arriving prompts queue until the source compaction and profile update finish. The home skips provider execution for leased and slice agents. Their execution worker validates the existing leased-profile request, reserves the backing agent, compacts its source run when needed, and commits the home-selected profile before acknowledging it. The same seam reconciles a home profile after a lost acknowledgement. Profile requests targeting Claude allow the native compaction deadline plus the normal relay response budget. Unchanged profile confirmations remain read-only during active turns. Headless/native-TUI runs do not enter the structured utility path. The utility is enqueued under the shared process-service lock and awaited after the guard is released.

The context figure uses the most recent assistant API call's input plus cache creation/read input. Output-only stream deltas retain that figure; the turn-wide result total still reports billed tokens. A result with no per-call input observation has unknown context rather than a guessed turn sum. This is internal parser state and preserves every serialized protocol shape. Compaction considers 75% of a strictly smaller effective window and honors `CLAUDE_CODE_DISABLE_1M_CONTEXT=1`.

Official references: [Claude Code model configuration](https://code.claude.com/docs/en/model-config#extended-context), [Sonnet context windows](https://code.claude.com/docs/en/model-config#sonnet-55-and-sonnet-5-context-window), and [API streaming usage](https://platform.claude.com/docs/en/build-with-claude/streaming). Round-5 CLI probes and test evidence live outside the repository under `/w/evidence/ctxswitch/round5/`.
