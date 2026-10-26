# Lane adapters (MP-08 / MP-09 / MP-10 / MP-11)

Run `node tools/lanes/lane-<operation>.mjs`, passing one JSON object on stdin.
Success writes one JSON object to stdout; failure writes `{error:{code,message}}`
to stderr and exits 1. `CHARIOX_BIN` selects the actual `chariox` executable.
The normal product access grant must already cover the coordinator process.
Provider login, credentials, transport and authority remain Chariox's job.

Prefer the updated CLI from the minimal-primitives change (protocol 615 source),
even with an older kernel. An older CLI can still collect bundles when its
kernel supports `StatKernelFile`: a `usage` refusal of `files get --sha256`
selects that compatibility path. Kernels that removed file metadata requests
require the updated CLI.

Every operation requires `kernel` and a stable `taskId` (letters, numbers, `.`,
`_`, `-`). A mutation's command id derives from this task id and its step.
Connection loss or request timeout gets one retry with identical bytes and id.
Refusals do not trigger retries or new prompts. Child stderr is never echoed.
Keep inputs, state and transferred artifacts in your own directory outside git.

| Operation | Additional input | Result |
| --- | --- | --- |
| `launch` | `alias`, `worktree`, `provider`, `profile`, `model`, `effort`, `permission`, local `brief`, absolute `remoteBrief`, local `stateFile`, epoch `deadlineMs` | Durable local receipt with Room, attachment, agent and submitted prompt; partial ownership survives a failed step. |
| `deadline` | `sessionId`, `agentId`, local launch `stateFile` | Foreground coordinator timer; waits, stops the lane, and records its result. |
| `steer` | `sessionId`, `attachmentId`, `agentId`, `expectedPromptId`, `prompt`, unique `steerId` | Kernel compare-and-steer response. A race remains a refusal. |
| `status` | `sessionId`, `agentId` | State, current prompt, last activity, coordinator deadline receipt and executing kernel capabilities. |
| `fetch` | `worktree`, local `directory` | Downloads `LANE_STATUS.md`, `PUSH_READY.md`, `ACCEPTANCE_UPDATE.json` with file hashes. |
| `bundle` | `sessionId` (operations Room), `worktree`, full `base`, optional expected `head`, absolute `remoteBundle`, local `localBundle`, existing local Git `repository` | Verified SHA-256, source HEAD/base ancestry, bundle tips, imported base ancestry and `refs/remotes/lanes/<taskId>`. |
| `slot` | `sessionId` (operations Room), `worktree`, absolute `slotScript`, string array `argv`, optional `timeout` seconds | Opens a Room terminal, runs the slot script, waits for its actual exit and closes the terminal. |
| `stop` | `sessionId`, `attachmentId`, `agentId`, optional local `pidLedger` | Cancels, destroys, verifies absence and cleans only exact ledger-owned processes. |

`launch` accepts optional `operationsAlias` to create an operations Room too. It creates a lane Room and attachment, refreshes the selected profile through
`RefreshProviderAccountProfile`, spawns the agent, uploads the brief with
`files put`, then submits its first prompt. Reuse the exact input and state
file to resume an interrupted launch; a lapsed attachment is renewed.
An uncertain submission first replays the original command bytes and identity.
The orchestrator always owns the deadline timer, on current and protocol 615
kernels alike. A resumed launch replaces any older deadline mode in its receipt.
Both report `deadline.mode:"coordinator"` and plainly say the lane has no kernel
deadline. Launch returns `coordinatorTimer.state:"required"`, its `argv` and JSON
`input`. Start that `lane-deadline` command as a **foreground coordinator job**
(the tool may yield a running-job handle), and keep the job alive while doing
other work. It waits until `deadlineMs`, stops the exact lane through the public
CLI, and writes `<stateFile>.deadline.json` with `waiting`, `stopped` or a typed
failure. A waiting receipt includes the exact coordinator timer PID for owned
cleanup. The timer is pending until this job is started. Never orphan it with
shell `&` or detach it: that loses the OS ancestry of the coordinator's access
grant. This is best-effort coordinator policy, requiring its process, network
and grant to remain available, and cannot replace a kernel hard stop after
coordinator/access loss. Stop/clean the job only by its verified owned PID when
ending early. Pass `stateFile` to status for the saved coordinator deadline;
without it the deadline time is unknown. Timer failures require restoring
access and an explicit unconditional `stop`.

`status` also accepts `waitForIdle:true`, `attachmentId`, `expectedPromptId`
and `timeout` (1–1800 seconds). It watches real CLI session events, requires
the expected turn's working-to-idle transition, then checks final state once.
An ended or expired replay stream fails explicitly. This is the current live
session stream, not the durable outside event subscription from EA2 §14.

`fetch` rejects short ids, duplicate ids, invalid/missing statuses, nonboolean
`new`, short commit hashes and missing date/evidence/note fields. It checks the
update format; matching existing ids and approving newly designed scenarios
remain the central acceptance tracker's responsibility.

MP-08 / MP-10: `fetch` reports an already consumed acceptance JSON explicitly;
other file errors and invalid rows still fail. `bundle` resolves the sole `HEAD`
tip from the verified bundle; a supplied expected head remains a strict guard.

`bundle` runs `sha256sum` in the same terminal command that creates the bundle,
then downloads with `files get --sha256 <hex>`. The CLI rejects changed bytes
before importing them. A missing or ambiguous terminal checksum fails clearly.
If an older CLI refuses that flag, `bundle` checks `StatKernelFile` against the
terminal checksum, downloads without the flag, and checks the local bytes too.
The tools use `events wait --timeout 0` for a nonblocking inbox read (exit 5 is
an empty timeout); only the older-CLI compatibility path uses file metadata.
Deadline timers belong to the coordinator.

`bundle` includes source history so it also works with an empty local Git
repository. A failed local ancestry check leaves only an incoming lane ref
for inspection; the final remote ref is not published until checks pass.
Neither bundling nor fetching runs a push or changes the local checkout.

PID ledger format: `{owner:taskId,processes:[{owner:taskId,pid,startTime,bootId,
executable}]}`. Record it when starting a process. `startTime` is Linux
`/proc/PID/stat` field 22; `bootId` is the current boot id; `executable` is the
exact `/proc/PID/exe` path. PID ≤ 1, missing ownership, recycled identity and
unknown executables are refused before cancelling anything. The whole ledger
is checked first and each identity is checked again immediately before its
signal. No names or process groups are signalled. A process still alive after
10 seconds yields `cleanup_incomplete` and retains its ledger; successful
cleanup moves receipts to `stopped` and clears `processes`.

Run regression checks with `NODE_OPTIONS=--max-old-space-size=4096 node --test
tools/lanes/*.test.mjs`. These checks supplement real two-kernel/provider drills;
they do not establish live acceptance.

`stop` is unconditional unless `expectedPromptId` is supplied. If a restart
removed its attachment, it records `cancelRefusal:attachment_not_found`, then
uses the kernel's agent destruction/cancellation path and verifies absence.
An expected-prompt guard is preserved: a missing attachment then refuses the
operation instead of destroying a potentially different turn. Already-removed
agents still permit exact owned PID cleanup. Other cancellation errors remain
refusals.

With `expectedPromptId`, `stop` only asks the kernel to cancel that exact turn.
It returns `stopped:false, agentRetained:true` and leaves the agent and PID ledger
alone, even if the worker already settled. Destruction and process cleanup have
no atomic prompt fence; request an unconditional stop separately only when all
work on that lane is meant to end.
