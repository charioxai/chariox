# Lane adapters (MP-08 / MP-09 / MP-10 / MP-11)

Run `node tools/lanes/lane-<operation>.mjs`, passing one JSON object on stdin.
Success writes one JSON object to stdout; failure writes `{error:{code,message}}`
to stderr and exits 1. `CHARIOX_BIN` selects the actual `chariox` executable.
The normal product access grant must already cover the coordinator process.
Provider login, credentials, transport and authority remain Chariox's job.

Every operation requires `kernel` and a stable `taskId` (letters, numbers, `.`,
`_`, `-`). A mutation's command id derives from this task id and its step.
Connection loss or request timeout gets one retry with identical bytes and id.
Refusals do not trigger retries or new prompts. Child stderr is never echoed.
Keep inputs, state and transferred artifacts in your own directory outside git.

| Operation | Additional input | Result |
| --- | --- | --- |
| `launch` | `alias`, `worktree`, `provider`, `profile`, `model`, `effort`, `permission`, local `brief`, absolute `remoteBrief`, local `stateFile`, epoch `deadlineMs` | Durable local receipt with Room, attachment, agent and submitted prompt; partial ownership survives a failed step. |
| `steer` | `sessionId`, `attachmentId`, `agentId`, `expectedPromptId`, `prompt`, unique `steerId` | Kernel compare-and-steer response. A race remains a refusal. |
| `status` | `sessionId`, `agentId` | State, current prompt, last activity and durable deadline receipt. |
| `fetch` | `worktree`, local `directory` | Downloads `LANE_STATUS.md`, `PUSH_READY.md`, `ACCEPTANCE_UPDATE.json` with file hashes. |
| `bundle` | `sessionId` (operations Room), `worktree`, full `base` and `head`, absolute `remoteBundle`, local `localBundle`, existing local Git `repository` | Verified SHA-256, source HEAD/base ancestry, bundle tips, imported base ancestry and `refs/remotes/lanes/<taskId>`. |
| `slot` | `sessionId` (operations Room), `worktree`, absolute `slotScript`, string array `argv`, optional `timeout` seconds | Opens a Room terminal, runs the slot script, waits for its actual exit and closes the terminal. |
| `stop` | `sessionId`, `attachmentId`, `agentId`, optional local `pidLedger` | Cancels, destroys, verifies absence and cleans only exact ledger-owned processes. |

`launch` accepts optional `operationsAlias` to create an operations Room too. It creates a lane Room and attachment, refreshes the selected profile through
`RefreshProviderAccountProfile`, spawns the agent, uploads the brief with
`files put`, arms its kernel deadline and submits its first prompt. Reuse its
exact input and state file to resume an interrupted launch. Use a fresh task id
and state file for different work. The deadline request is required: there is
no client timer fallback. The executing kernel needs `SetAgentDeadline` and
`GetAgentDeadline` (provisional local 574 on the amint union; final allocation
belongs to the coordinator). A deadline can shorten but cannot extend.

`status` also accepts `waitForIdle:true`, `attachmentId`, `expectedPromptId`
and `timeout` (1–1800 seconds). It watches real CLI session events, requires
the expected turn's working-to-idle transition, then checks final state once.
An ended or expired replay stream fails explicitly. This is the current live
session stream, not the durable outside event subscription from EA2 §14.

`fetch` rejects short ids, duplicate ids, invalid/missing statuses, nonboolean
`new`, short commit hashes and missing date/evidence/note fields. It checks the
update format; matching existing ids and approving newly designed scenarios
remain the central acceptance tracker's responsibility.

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
