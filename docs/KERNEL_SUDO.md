# Sudo windows (protocol 460)

MP-08 / MP-10 / MP-11 A04. In a Chariox terminal, focus a local regular agent
and enter `/sudo <prompt>`. Every connected host terminal receives one passkey
popup naming the agent, the full prompt and the window; guests cannot answer
it. Press Tab to choose 1 hour (default), 2, 4 or 8 hours, then enter the
passkey. The critical-approval remember window never satisfies sudo. Never type
a passkey into an agent transcript, provider prompt or shell command.

The window starts at that fresh verification and covers only the work the
owner authorized. With `CHARIOX_ROOM_AGENT_TOOLS=1` this is the prompt's durable
task: the agent keeps `chariox_kernel_request` across its yields, waits and
kernel-correlated continuations (delegate, workflow or watcher results, owner
resume, corrections, the expiry re-evaluation). Without durable tasks the
window covers the first turn only. Each privileged call still needs a running
turn of that work bound to the live window; the kernel rechecks before effects.

While the work is open the agent is held for it (the causal fence). Unrelated
prompts, peer messages and other inbox events never steer or enter the
elevated context: they stay queued with a visible "Deferred" notice and run as
distinct regular turns once the work ends, the window expires or it is revoked.
Spawned, forked and workflow agents never inherit the window.

If a warm provider must relaunch to discover the sudo tool, it has 60 seconds
from reload to become ready, including its launch delay. A failed or stalled
relaunch returns a typed kernel-access error saying "provider relaunch failed",
ends the window as `refused_or_cancelled` and releases the held work. Retry
`/sudo` after the provider is available; the failed request leaves no elevation.

A native provider may be idle while its catalog operation lane is still busy.
The first sudo turn waits for the refresh to complete, keeping its work hold.
The 60-second budget counts only idle refresh attempts, including deferred
retries. Ordinary work resets it. A locked Chariox vault is unlocked before
the budget starts, so its passphrase and duration popups never count; they
keep their own expiry, and a window revoked meanwhile closes them. Each retry
re-checks the vault, so a relock between retries prompts again and pauses
the budget until answered. Budget
exhaustion, a refresh failure or an unanswered vault popup returns a typed
kernel-access error saying "provider catalog refresh failed", ends the window
as `refused_or_cancelled` and releases the held work.

The initial window permits owner session inventory. Additional typed operations
require a fresh owner passkey in an operation-scope popup showing their exact
parameters. That approval binds a digest to the original work and window;
changed parameters or targets require a new approval. Scope is not inferred
from natural-language similarity or a delegate's result. Extend renews only
time. Destructive agent/workflow operations retain the immutable direct-creator
fences even under sudo. Raw credential values and full kernel configuration
are unavailable to this bridge.

Every attached terminal shows a sudo row with the remaining time, the absolute
expiry and `[Extend]` / `[Revoke]`; the row reads the kernel's deadline from the
session snapshot, so reconnecting terminals show the same window.

- `/sudo status` lists live windows; `/sudo extend [sudo:<id>]` opens one fresh
  passkey popup for the same work and sets the expiry to the verification time plus the chosen
  duration (never banked time); `/sudo revoke [sudo:<id>]` ends it and
  interrupts its running turn. `/kernel access list|revoke` still work.
- At 10 minutes left the kernel warns every terminal once per window revision;
  an extension resets the warning.
- On expiry the window ends, its running turn continues as regular work, a
  waiting task receives a regular re-evaluation event through the wake inbox,
  and the deferred work runs. Enter `/sudo` again to re-elevate.
- Restart, passkey rotation, revoke, session end, agent removal or a placement
  change end the window (fail closed). After a restart a notice says so and
  open work continues regularly until the owner reauthorizes.

Each window's timer proves it is live: the scheduled task records its own
revision, and the kernel pump (the dead-man) enforces any warning or expiry the
timer misses and alerts every terminal instead of staying silent. Failed warning receipts and waiting-work expiry wakes are retried; restart replay restores an undelivered expiry wake without restoring elevation. Authority
never depends on a timer: every use compares the monotonic deadline, so neither
a late timer nor a wall-clock change can extend a window.

Sudo never answers approvals: `RespondToInteraction` is refused to
`chariox_kernel_request` and to every kernel-agent caller, and the legacy
`chariox.meta.resolve_runtime_interaction` tool is removed. Owners answer
approvals in their terminal. Sudo also cannot mint access or sudo, answer
credential-entry prompts, export secret values, change the passkey or
configure kernel access, pairing, relay or Cloud identity.

`chariox access list` prints a JSON object with `grants` and `sudo_turns`
arrays (each sudo entry is a window). `kernel_access.sudo` records requested,
authorized, started, timer_armed, warning, extension_requested, extended and the
end reason (expired, work_ended, explicit_revoke, restart_dropped, …). Receipts
contain no passkey.

A sudo turn's `chariox_kernel_request` takes one serialized
`LocalDaemonRequest`, for example:

```json
{"request":{"ListSessions":null}}
```

External agents holding a process-bound local-kernel grant can request a window
over that kernel's Unix socket:

```sh
chariox sudo request --agent <agent-id> --prompt "<full prompt>" [--socket /absolute/kernel.sock]
```

The host's popup names the grant holder's OS executable and PID, the target
agent and session, and the full requester-supplied prompt. Only the host's
terminals can answer it; the external client never sends or receives the
passkey. TCP, relay and ungranted peers are refused; any local session's agent
may be targeted, and the window ends when the requester's grant ends. Shell CLI
calls use the tracked provider's OS process tree. For a spawn launcher, the
kernel also records the unique OS-verified endpoint server in that tree before
prompt dispatch. Only descendants born after the current elevated turn binds
are admitted below that server; other pre-turn children receive no exemption.
A descendant retained from an
earlier turn has no authority in a later turn. Unix terminal subscriptions are
limited to the window session and owner attachments.

A leased agent (running on a worker kernel) is elevated by its home kernel only
(MP-08/MP-09/MP-10/MP-11 A10). The window is bound to the agent's execution
lease; any other placement ends it. Before each elevated leased turn, and on
Extend and at the end, the home sends the worker `UpdateLeasedSudo` (relay
peer protocol 83) naming the exact leased home prompt, the window revision and
the time left. The worker lists `chariox_kernel_request` only for that turn
and forwards every call home, where the same window, turn and lease are checked
again before any effect; the worker fence can only narrow authority. Other
leased agents on the worker, including ones the elevated agent delegates to,
never see the tool. The fence lives in worker memory: a worker restart drops it,
and the next renewal for that window ends it at home (`worker_restarted`).
Workers older than peer protocol 83 are refused, `/sudo` entered on a worker
for a leased backing agent is refused, and shell CLI elevation stays local-only.

Run `scripts/kernel-access-sudo-drill.sh` on the Linux builder for the source
regression drill; real acceptance uses the built TUI, kernel and an official
provider (see the lane evidence).

MP-08 / MP-10 / MP-11: the shared router also filters every response to a
local grant or sudo MCP caller. Credential read and mutation replies redact
literal injection values. Raw MCP/connector configuration, native login
output/codes, pairing/Cloud admission credentials and enrollment callbacks
are withheld; inspect those through the host terminal. An operation can complete
while its reply is withheld; check host-terminal state before retrying. Executor failures use
a value-free error because parser/provider diagnostics can echo secrets. Exact
constant refusal, expiry and revocation diagnostics remain available.
This preserves normal workspace/history authority and is not a file sandbox.
