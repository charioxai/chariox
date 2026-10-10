# MP-08 / MP-10 / MP-11 — Mac owner acceptance kit

Run after tonight's coordinator-controlled q restart, with the owner present.
This is the human gate for **sudo validated**. PR #873 (`/meta` retirement,
local 430) stays HOLD until this kit and the Linux automated matrix pass and
the coordinator confirms sudo validation. The owner approved retirement only
at the end of the program. Do not merge retirement to run this kit.
Kernel Access plan §7/§9.2 PRs 3, 5, 7, 8, 10 and D1–D16 are the authority;
`KA_SUDO_CHECK_MATRIX.md` maps every check to automated proof and remaining UI
proof. This kit does not close MP-10 fresh-machine comparison or substitute
for MP-11 independent security review.

Use a dedicated disposable acceptance session/workspace on the live kernel.
Use official provider/account paths already selected by Chariox. Do not copy
profiles/auth files, export a real vault, dump daemon/controller configuration,
or type passkeys into commands, agent prompts, kit stdin or transcripts.
The kit reads no vault; only the human-operated product popup gets a passkey.
Linux uses existing synthetic cfg(test) vaults, never the live Mac vault.

## MP-08 / MP-10 / MP-11 — H0 preparation and identity

Record live kernel release/source, local/relay protocol, OSS/Cloud commits,
provider/model, macOS, terminal builds, source/binary hashes and UTC time.
Expected Linux baseline is `97d7d9e20` on `74e50b787`, local 416 / relay 70.
The Mac aggregate may have a newer assigned protocol: record it separately;
never relabel builder results as validation of that aggregate. Verify #872's
behavior is included, and run the automation again on a changed security seam.
Build `@chariox/kernel-client` and `@chariox/shell` from the selected source
before opening the kit. The helpers require built JS, Node 22 and Python 3.

Set these public identifiers in an owner OS shell (substitute actual values):

```sh
export KA_CHECKOUT=/absolute/path/to/reviewed/chariox
export KA_EVIDENCE=/Users/miguel/.codex/evidence/browser-resume-20260930/kasudo-validation/mac-<UTC-run>
export KA_ENDPOINT=ws://127.0.0.1:<live-kernel-port>
export KA_SESSION=<disposable-session-id>
export KA_AGENT=<focused-official-provider-agent-id>
export KA_SOCKET=<absolute-live-Unix-socket-path>
export KA_DB=<active-CHARIOX_HOME>/kernels/<kernel-id>/state.db
mkdir -m 700 -p "$KA_EVIDENCE"
```

Get kernel/session/agent IDs from product status, not credential files.
The default socket may be calculated with the public
`defaultKernelUnixSocketPath(kernelId, activeHome)` in
`packages/kernel-client/dist/kernel-unix-socket-path.js`; explicit configured
socket overrides require the product-reported path. `KA_DB` above is the
ordinary default; use the actual public configured state path if overridden.
Observer/access-list commands must inherit the live kernel's `CHARIOX_HOME`
so the normal first-party client finds its local credential. Do not display it.

```sh
python3 "$KA_CHECKOUT/scripts/sudo-kit-receipts.py" --database "$KA_DB" --checkpoint > "$KA_EVIDENCE/checkpoint.json"
node "$KA_CHECKOUT/scripts/sudo-kit.mjs" tcp-check --checkout "$KA_CHECKOUT" --endpoint "$KA_ENDPOINT" > "$KA_EVIDENCE/tcp.jsonl"
node "$KA_CHECKOUT/scripts/sudo-kit.mjs" observe --checkout "$KA_CHECKOUT" --endpoint "$KA_ENDPOINT" | tee "$KA_EVIDENCE/observer.jsonl"
```

Expect two 401 receipts (missing/wrong credential). Keep the observer open in
its own shell; it captures popup IDs/kinds/lifetimes and empty-popup events,
without popup input or requester text. It is an additional owner protocol
observer, not a substitute for visible UI. Type `exit` to close it.

Connect owner local TUI **A**, owner Web **B**, owner remote TUI **C**, and an
owner terminal **D** in the waiting room/unattached to this session. If native
clients advertise sudo support, include them; otherwise record explicit N/A.
For shared-session checks add a real invited guest terminal **G**. Record
which terminals are attached and which are waiting. Keep harmless providers
and sessions isolated from useful work.

## MP-08 / MP-10 / MP-11 — H1 popup fan-out and first answer

1. Focus KA_AGENT and enter `/sudo Report only the acceptance marker KA-H1.`
   All A/B/C/D owner terminals receive one kernel popup with the same interaction
   ID, agent, session and **full prompt**; G sees none. The agent must not start.
   Current TUI focus policy announces a pending prompt; F8 opens its protected
   popup without taking composer keys. Check arrival and then open it in each
   TUI. Record actual Web/remote behavior and any mismatch with the owner's
   required popup visibility; protocol arrival alone cannot pass rendered UI.
2. Owner enters one wrong value in A. Expect rejection, popup remains open
   everywhere, no provider dispatch. Do not run a live five-failure lockout.
3. Owner enters the correct passkey in B. Expect exactly one provider turn;
   A/B/C/D close the same popup, including D. Observer emits an empty set.
   The winning terminal appears in `kernel_access.sudo` attribution.
4. Repeat with a fresh marker, answer correctly in A and B nearly together.
   Expect one winner/one start, other answer “already answered” or closure
   before it can be sent. No duplicate turn and no second authorization.
5. Repeat and click **Refuse** in C with no passkey. All popups close; agent
   never receives the prompt. Fresh `/sudo` after any remember-window critical
   approval must still require a fresh popup/passkey.
6. Owner pastes a passphrase only into the protected popup field. It is kept
   exactly or clearly refused, never normalized or rendered in history.
   Capture screenshots before entry and after resolution, not entered values.

Receipts: popup IDs and close events for every terminal; refused/rejected then
one started entry; provider prompt/run IDs; winning terminal. Wrong attempts
remain audits, never the passkey value.

## MP-08 / MP-10 / MP-11 — H2 shared-session host authority

Have G request/attempt to authorize the acceptance interaction through the
normal guest UI. Expect no host popup on G and refusal of any forged answer.
Repeat H1 while a host terminal is unattached. Only host A/B/C/D can grant;
guest full collaboration never becomes grant/critical/sudo authority. Record
guest refusal and host-only popup screenshots. Don't reclassify an owner
observer as a guest by passing arbitrary local user IDs.

## MP-08 / MP-10 / MP-11 — H3 persistent external process and descendants

In an OS shell outside Chariox's provider process tree, run:

```sh
node "$KA_CHECKOUT/scripts/sudo-kit.mjs" holder --checkout "$KA_CHECKOUT" --socket "$KA_SOCKET" --session "$KA_SESSION" --agent "$KA_AGENT" --minutes 6 --chariox /absolute/path/to/chariox | tee "$KA_EVIDENCE/holder.jsonl"
```

Expect A/B/C/D to show **external access**, with the actual Node executable,
the printed holder PID, target session, six-minute term and requester text
labeled as such. No agent or kit prompt asks for the passkey. Refuse once,
then rerun and approve in an owner popup. Keep this process open: a short-lived
`chariox access request` process alone would end its grant on exit.

In the holder type `probe`, `child`, `list`, then `subscribe`. Expect success for
the holder and its OS child; `list` contains only KA_SESSION; subscribe starts
the real session stream. Type `foreign-probe <other-session-id>` and expect
refusal. In an
independent sibling OS shell run:

```sh
node "$KA_CHECKOUT/scripts/sudo-kit.mjs" probe --checkout "$KA_CHECKOUT" --socket "$KA_SOCKET" --session "$KA_SESSION" > "$KA_EVIDENCE/sibling.jsonl"
```

Expect a nonzero exit and `kernel_access_denied`. Public grant ID knowledge
does not authorize the sibling. Holder cannot answer a pending critical:
type `critical <actual-interaction-id>` while H6's harmless critical is pending;
expect refusal, approval stays pending. No passkey is offered to the holder.
PID-reuse mutation and PGID-join adversarial checks are deterministic Linux
fixtures; do not force PID exhaustion or alter unrelated Mac processes.

## MP-08 / MP-10 / MP-11 — H4 extension, expiry, exit and revoke

1. Leave the six-minute holder open. Around one minute, the five-minute-before-
   expiry **Extend access** popup appears on all owner terminals. Refusal needs
   no passkey; approval needs a fresh one and starts the selected new term.
   Record expiry before/after using `access-list` below. No remember reuse.
2. In a fresh six-minute run refuse extension and wait for real expiry. The
   idle subscription stops, access list loses the grant, `probe` and `child`
   refuse. The protocol client may attempt reconnect, but no protected events
   or replay snapshots can arrive after authority ends. Record last delivered
   event/revocation times; generic connection events are not session data.
3. Repeat grant, type `subscribe`, and revoke only its ID from owner TUI:
   `/kernel access revoke <grant-id>`. Expect immediate cutoff and denied probe.
4. Repeat grant/subscription, type `exit`. Expect holder grant disappears even
   if its descendants were subscribing. Repeat using normal close/archive of
   the disposable session: grants/queued sudo disappear and running sudo stops.
5. Request external sudo without owner answer, then revoke the holder while
   pending; popup closes everywhere and requester receives refusal/cancellation.
   Run the no-terminal timeout on a separate acceptance kernel if disconnecting
   all live owner terminals would disturb useful work. Record configured timeout
   and exact error. Builder fixtures already cover deterministic expiry.

```sh
node "$KA_CHECKOUT/scripts/sudo-kit.mjs" access-list --checkout "$KA_CHECKOUT" --endpoint "$KA_ENDPOINT" >> "$KA_EVIDENCE/access-list.jsonl"
```

Do not use `revoke --all` on unrelated live grants. H7/H9 require a coordinator-
owned acceptance kernel if useful grants/turns exist on the main kernel.

## MP-08 / MP-10 / MP-11 — H5 official provider, external sudo and shell CLI

In a granted H3 holder, type `cli-sudo <harmless full prompt>`
(single line stdin; include visible punctuation/Unicode). This launches the
actual `chariox sudo request --agent ... --prompt ... --socket ...` as a holder
descendant. A/B/C/D get a **sudo** popup naming OS requester, target/session
and exact full prompt. Refuse once (CLI nonzero), repeat and approve (CLI zero,
one ordinary provider turn elevated). Also exercise `request-sudo <prompt>`
through the product client, not just the OS CLI.

Create an evidence-independent scratch directory with `mktemp -d`; place
`session list` plus newline in `command.chx` there. Enter `/sudo` in the TUI
asking KA_AGENT to edit a harmless acceptance file and run, from its own shell:

```sh
node /absolute/reviewed/chariox/apps/shell/dist/shell.js run /absolute/scratch/command.chx --kernel-url ws+unix:///absolute/live/kernel.sock
node /absolute/reviewed/chariox/scripts/sudo-kit.mjs probe --checkout /absolute/reviewed/chariox --socket /absolute/live/kernel.sock --session <acceptance-session-id>
```

Both must succeed **during this exact turn**, with normal provider shell tools.
No bearer or token env/file may be supplied. Let the turn yield completely;
ask the next ordinary turn to repeat the identical commands. Both must refuse
with no live authority (CLI nonzero). Record exact provider/run/prompt IDs and
completion time. Provider tools and transcript remain usable. Repeat the
official harness leg for Codex, OpenCode and Claude selected through product
accounts; record failures individually and macOS Seatbelt launch result.

## MP-08 / MP-10 / MP-11 — H6 owner-only decisions and no inheritance

Use an owner decision genuinely needed by the real provider's authorized work
(for example a blocked review that needs a missing real account or deployment
resource). Record the actual interaction ID. Alternatively use an authorized
critical operation in a real installed App on a real service/account. Synthetic
App actions are supplementary regressions and never live acceptance.

During the human-authorized sudo work, ask the agent to answer that interaction
through `chariox_kernel_request`. Expect refusal for approval, refusal, Resume
and Cancel alike: sudo agents cannot answer owner interactions. The original
interaction stays pending and the action has no effect. The owner answers
through the real terminal popup with a fresh passkey when required; expect one
effect and all popups to close. Spawn a second ordinary agent for related real
work; it receives no elevation and cannot answer the owner interaction either.
The external holder's `critical` command also refuses.

Do not manufacture an owner decision or invent a low-level critical fixture on
the live Vault. If the real work has no needed decision and the real App/account
is unavailable, record this leg BLOCKED with the exact missing resource. Other
kit steps continue; the missing leg cannot be declared accepted.

## MP-08 / MP-10 / MP-11 — H7 interrupt, busy queue and revocation

Start a bounded harmless official-provider task held at a scratch-file gate.
Authorize `/sudo` while busy. Current turn gets no sudo/steering; pending sudo
is separate in access list and doesn't appear in durable prompt backlog.
Revoke its entry (or revoke all on the dedicated acceptance kernel), release
the gate, and verify no sudo dispatch. Fresh `/sudo` requires fresh passkey.
Repeat, allow the queued sudo turn to start, then interrupt it through the
normal UI. Unix/MCP authority disappears immediately; any still-running owned
helper can't read more session data. Next ordinary turn remains unprivileged.
For external queued sudo revoke the source grant before busy turn finishes;
expect cancellation and no stale dispatch. Record each entry and prompt ID.

## MP-08 / MP-10 / MP-11 — H8 restart drops queued authorization

On the coordinator-owned acceptance kernel, authorize sudo behind a busy turn.
Capture access list and durable prompt queue before restart. Coordinator uses
the standard kernel/q restart mechanism, never a PID-name kill. Reconnect
A/B/C/D and the holder: no grant, no queued/running sudo, no privileged prompt
replay. A dropped-authorization notice exists without prompt content. The
durable ordinary prompt/session history remains coherent. Fresh request needs
owner passkey. Do not restart shared useful infrastructure just to run H8.

## MP-08 / MP-10 / MP-11 — H9 owner-operated passkey rotation

Owner uses `/credential vault manage`, then **Change passphrase**, on the
designated acceptance kernel, through the existing protected current/new
input flow. Owner controls retention/recovery of the new passphrase; helpers
never receive it. Have an external grant, a busy queued sudo and a running sudo
in separate disposable sessions immediately before rotation. Expect every
grant revoked, remember windows ended, queued sudo cancelled, running sudo
interrupted, idle subscriptions cut off. New sudo/grant with old passkey fails;
new passkey succeeds through a fresh popup. Record only rotation outcome and
revocation/interruption identifiers. If live vault rotation is not scheduled,
use a separate owner-managed private acceptance vault/stack and state this
limitation. Do not silently skip or claim live-vault rotation from builder tests.

## MP-08 / MP-10 / MP-11 — receipts, cleanup and acceptance

Read `after_sequence` from checkpoint.json and substitute its numeric value:

```sh
python3 "$KA_CHECKOUT/scripts/sudo-kit-receipts.py" --database "$KA_DB" --after-sequence <checkpoint-number> > "$KA_EVIDENCE/receipts.jsonl"
```

This opens SQLite read-only and selects individual allowlisted public fields;
it never dumps payloads, pins, commitments, keys, credentials or transcripts.
Check event sequence/time, grant/session/holder identity, outcomes and winning
terminal. Join critical receipts by `turn.entry_id` to the started sudo event;
verify matching agent, provider_run_id and exact prompt_id, plus target and
choice. Keep UI screenshots redacted and retain no input values.

Type `exit` in kit clients, revoke only acceptance IDs, end acceptance sessions,
stop only owned provider runs through product controls, and remove only their
scratch directories. Keep live owner/provider profiles and vault intact.
Record exact owned resources and cleanup outcomes; no group/name-wide signals.

Write one result per H0–H9 and matrix row: PASS, FAIL (first seam), BLOCKED or
N/A with reason. Attach public receipts, source/release identities, redacted
screenshots for A/B/C/D/G, official-provider traces, timestamps and cleanup.
**Sudo validated** requires all applicable rows, owner UI/passkey and official
provider legs, consistent receipts, and clean teardown. A missing client,
critical fixture, restart or rotation leg remains an explicit acceptance gap.
Only then may the owner confirm #873 retirement. The coordinator publishes;
this lane performs no push, merge, deployment or Cloud change.
