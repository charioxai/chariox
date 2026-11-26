# Account machine limits: the kernel half (row 569)

Status: built on the L14 head (protocol 1015 / relay peer 452). Cloud half: c#852 (account limits in the database, the account page, `limits` / `set_limits`), plan D16 in the Cloud repository. Cloud-only controls stay in Cloud; nothing here imports Cloud code.

## What the person gets
The account has two limits, set by the owner on the Cloud account page: "Machines at once" and "Spend per day" (account currency). Agents read them, lower them directly, and raise them only with the person's say-so naming the exact change. The no-ask limits (`agents.offload_*`, c#701) can never exceed the account limits.

## Surfaces
1. **Control-plane port.** Two new `ControlPlaneRequest` variants, the same shapes c#852 accepts (`apps/api/src/contracts/control-plane.ts`):
   - `Limits {}` answers `{kind:"limits", machines_at_once, spend_per_day_micros, currency, version}`.
   - `SetLimits {expected_version, machines_at_once, spend_per_day_micros, approval?}` answers the same shape. `approval` is `{kind:"sudo"|"card", ref, from:{...}, to:{...}}`; Cloud applies a raise only when `from` is its current value and `to` is exactly the new value. The kernel credential, not a signature, authenticates the call; `ref` is the card's interaction id or the sudo entry id, kept for the audit.
   - The capability's `actions` list carries `limits` and `set_limits`; a kernel whose service does not list them offers no limit tools (a self-hosted kernel has no account limits and says so).
2. **Agent tools and CLI.** `chariox.machines.limits` (read) and `chariox.machines.set_limits`, on demand like every tool outside the four always-loaded ones, and listed only while the kernel has a connected account (no account, no limit tools; the context budget numbers do not move). The CLI is one command, `chariox machine limits [--scope account|no-ask] [--machines N] [--spend AMOUNT]`: bare it reads, with a change flag it sets (the agent guide is capped at 145 lines, so it takes the two lines of one command, offset by printing a command's note on its summary line); it reaches the kernel as `MachineAcquisition` `limits` / `set_limits`, the same path as `machine acquire`. `set_limits` takes `scope: "account" | "no_ask"`. Account scope goes to Cloud; no-ask scope is the kernel's own `agents.offload_*` config. `limits` returns both sets and the effective no-ask limits after the clamp.
3. **Settings > Machines** shows the account limit as the outer bound and labels a clamped no-ask value; it reads it through the same port, never through Cloud code.

## Authority (kernel-enforced, not only the UI)
For an agent-originated `set_limits`, per field:
- **Lower** (new value below the current one): applied directly, no card.
- **Raise**: needs one of
  - **(a) a /sudo grant naming it** (for the account limits and for the no-ask limits alike: the grant names a number, not a scope). The kernel parses the owner's own `/sudo <prompt>` once, at submission (`policy::parse_sudo_prompt`), with a strict grammar: `raise limits to <N> machines` and `raise limits to <amount> <CUR> a day`, optionally both joined by `and`. The parsed result is stored on the sudo window as structured `limit_grant {machines_at_once?, spend_per_day_micros?}` (never re-parsed from display copy). It covers exactly what it names: a raise to a value at or below the named one passes; a field it does not name, or a value above it, is not covered. The agent must be the one the live window is bound to (`sudo_for_run`); the approval sent is `kind:"sudo"`, `ref` = the entry id, `to` = the exact value the agent asked for.
  - **(b) a card.** The kernel raises a runtime interaction "Raise the account limit" in the agent's Room: description `Machines at once 2 -> 4` (and/or `Spend per day 10.00 EUR -> 15.00 EUR`), choices Approve / Deny, shown to every client in the session, answered only by the person (not the requester or its sub-agents), same pattern as `approve_machine_acquisition`. Approve sends `kind:"card"` with the exact `from`/`to` read when the card opened; if the limit moved meanwhile Cloud refuses, the kernel tells the agent to read again. Deny changes nothing and the agent gets the reason. A card for a no-ask raise names `agents.offload_*` old -> new and applies to the kernel's config on Approve.
- A raise with neither is never sent to Cloud; the refusal says which two ways exist.
- Sub-agents and peers cannot approve for their parent; a card unanswered is not a refusal and not an approval (it stays open like the acquisition card).

## The clamp (C6)
Effective no-ask limit = min(the person's no-ask value, the account limit), per field. The kernel reads the account limit through `Limits` when it admits a no-ask acquire (fresh read; a failed read refuses the no-ask path and falls back to the ordinary card, it never allows). Above the clamp the kernel refuses the no-ask acquire with the account reason ("the account allows 3 machines"), and Settings labels the value as clamped. The Cloud admission check still applies behind it.

## Files (after the L14 union)
`local/api/types/control_plane.rs`, `runtime/control_plane.rs` (new variants), `runtime/state/machine_acquisition.rs` + new `machine_acquisition/limits.rs` (tools, card, authority) + `without_asking.rs` (clamp, #1471), `runtime/state/sudo/policy.rs` (grammar) and `KernelSudoTurn.limit_grant` (`local/api/types/kernel_access.rs`), `bin/chariox-cli/arguments.rs`, `packages/kernel-client` (types, `ipc-control-plane-requests.ts`, Settings read), protocol shapes and `fixtures/protocol-versions.json`. The protocol bump is the last commit (local 1017, peer 454: the L15 union took 1016 / 453).

## Cells
| Cell | Check | Where |
| --- | --- | --- |
| C2 | agent tool and CLI read the account limits and the no-ask limits; they equal the Cloud page and Settings | Rust tool test; drill against a local Cloud stack |
| C3 | agent lowers the account limit and a no-ask limit: applied, no card | Rust test with a fake port; drill |
| C4 | raise of the spend cap with no /sudo: a card with the exact old -> new values on every client; Approve applies exactly; Deny leaves it and the agent gets the reason; requester and sub-agent answers are refused | Rust card tests; drill with two attached clients |
| C5 | same card path for machines at once and for a no-ask limit above its value | Rust tests; drill |
| C5s | `/sudo raise limits to 4 machines`: the agent raises to 4 with no card; 5 machines, and the spend cap, go to a card; a window that ended or belongs to another agent covers nothing | Rust grammar and authority tests; drill |
| C6 | a no-ask value above the account limit is clamped and labelled; the excess no-ask acquire is refused with the account reason; a failed read never allows | Rust tests; web test for the label; drill |
| C7 | no Cloud-only control or Cloud import in OSS | oss-boundary-guard green |
| P | local `/sudo raise limits to 4 machines` text parsing rejects look-alikes ("lower", "raise limits to four", extra words, two numbers) | Rust grammar tests |

## Tests
Built with the code: `machine_acquisition/limits_tests.rs` (tool reads, lower, raise card, deny, stale card, no-ask raise, forged approval, clamp, failed read, no account limits), `sudo/limit_authority_tests.rs` (sudo cover, beyond the grant, window ended, lower, the person's own change, stale version, the owner's prompt names the grant), `local/api/tests/protocol_shapes` (request, response and approval shapes), `apps/cli/src/machine-command.test.ts` and `packages/kernel-client/src/offload-setting.test.ts`. The Settings label is in the Cloud web (Settings > Machines, c#852).

**Written first, green** (`runtime/state/sudo/limit_grant.rs`, self-contained; the code PR adds `mod limit_grant;` and `KernelSudoTurn.limit_grant`): the grammar and the cover rule (cell P and the cover half of C5s): the named machines, spend, both in either order, 20 look-alikes and longer sentences that grant nothing, and `covers` (the named value and below pass; beyond it, an unnamed field, another currency and a partly covered raise do not).

**Drop in as RED tests when L14 merges** (names fixed here; each fails on the L14 head and passes with the code):
- `machine_limits_tool_reads_account_and_no_ask_limits` (C2): fake port answers `limits`; the tool returns both sets and the effective no-ask value; a service without `limits` in its actions yields "no account limits here".
- `agent_lower_is_sent_without_approval` (C3): a lower of each limit reaches the port with no `approval`; no card is opened; the no-ask lower changes the config only.
- `agent_raise_opens_a_card_with_exact_values` (C4/C5): a raise opens one interaction whose text and payload carry old and new; the requester and a sub-agent cannot answer; Approve sends `approval{kind:"card", ref: interaction id, from, to}` equal to the card's values; Deny sends nothing and the tool answer carries the reason.
- `agent_raise_card_goes_stale_when_the_limit_moves` (C4): the port answers a refusal after the card; the tool answer says to read again; nothing is applied.
- `sudo_grant_raises_without_a_card_exactly_as_named` (C5s): a live window with `limit_grant{machines:4}` lets 3 -> 4 through with `approval{kind:"sudo"}` and no card; 3 -> 5 and the spend cap open a card; an ended window, another agent's window, and a grant parsed from the agent's own text cover nothing.
- `no_ask_limits_clamp_to_the_account_limit` (C6): effective = min per field; an acquire above it is refused with the account reason; a failed `limits` read refuses the no-ask path and leaves the ordinary card; the Cloud refusal still applies.
- `set_limits_shapes_round_trip` and the protocol shape files for `limits`/`set_limits` (bump 1016/453, last commit).
- Web (`packages/kernel-client`, Settings): the clamped label and the account bound read through the port (C6).

## Drill (local Cloud dev stack + drill kernel)
1. Start the Cloud dev stack on c#852's head and a drill kernel enrolled to the owner's account; give the drill kernel its Claude account with the setup-token route; run a real agent in a Room.
2. Owner sets 4 machines / 10 EUR on `/account/limits`. The agent runs `chariox machines limits`: same values (C2).
3. Agent lowers machines to 3: applied, no card (C3). Agent raises the spend cap to 15 EUR: a card shows `10.00 EUR -> 15.00 EUR` on two attached clients; deny, then ask again and approve; the page shows 15.00 EUR (C4).
4. Owner types `/sudo raise limits to 4 machines`; the agent raises 3 -> 4 with no card; it asks for 5: a card (C5s).
5. Owner sets no-ask machines to 6 in Settings: shown as clamped to 4; a no-ask acquire is refused with the account reason (C6).
6. Every run arms a kernel wake (timer or process watch); no step ends a turn with work pending.

## Open points for the owner
- The grammar names only machines and spend; other phrasings need a card. Say if more phrasings should count.
- The card is answerable by the person only; if the person wants any attached client to answer, that is the same rule as the acquisition card.
