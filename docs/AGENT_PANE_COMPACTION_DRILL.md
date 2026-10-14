# MP-08 / MP-10 / MP-11: OpenCode compaction continuation drill

The kernel remains the prompt and interaction authority. OpenCode owns its
conversation and automatic compaction. Its documented native message API can
report a synthetic continuation user after an automatic compaction summary,
including when a Chariox prompt arrived while the preceding turn was compacting.
The continuation's assistant parent differs from the submitted provider user id.

The adapter correlates that continuation only inside the same native session,
with an automatic compaction trigger and a summary completing after the accepted
user was created. The synthetic text parts must identify that continuation user
and session. An intervening ordinary user, a manual compaction, stale summary,
foreign session or baseline message cannot extend the accepted turn. Provider
compaction summaries stay out of the user transcript and cannot settle the turn.
The original kernel prompt and native permission interaction origin stay intact;
continuation parents expire when the prompt is replaced or cancelled.

This reads existing provider-native fields; it changes no Chariox wire shape.
Local protocol 508 / relay peer 116 continue to cover the pane history changes.

Run the real PANE-06/PANE-08/PANE-14 path: a product-linked OpenCode Go agent,
normal source-review turns in the built web client and TUI, the real kernel and
hosted `wss` relay, and the physical WAN client at DPR 1 and 2. Grow genuine
history until native automatic compaction starts; enter the next source-review
prompt while it is compacting. Record provider message ids/timestamps and
synthetic/automatic flags, kernel sequence order, browser/TUI captures and
socket timing without credentials. Before the fix the completed native answer
can remain absent while Chariox says working. After the fix the answer streams,
settles once, and retains its order after reload; the compaction context and
synthetic user are not extra user turns. Exercise required permission mode too:
a continuation tool approval must use the original kernel runtime interaction.

Compare the native SQLite/JSON observation path as well as the live adapter.
Both must omit `synthetic: true` user parts and assistant `summary: true`
messages, including their completion telemetry. A user's summary object or
ordinary text saying "Continue" must remain visible. Count actual user prompts
against kernel turns across compaction and verify the final public answer in
the real UI. A model may omit a requested reference after compaction; preserve
that failed criterion and distinguish it from missing output or a stuck turn.

A controlled request to the documented native summarize endpoint is useful
supplementary reproduction evidence. It does not replace natural long-history
acceptance or the full provider/client matrix. Focused Rust tests are regression
checks only. Record each source identity and binary hash independently; passing
source checks cannot close the live acceptance rows.
