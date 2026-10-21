# Sudo windows (protocol 484)

MP-08 / MP-10 / MP-11 A04. In a Chariox terminal, focus a local regular agent
and enter `/sudo <prompt>`. Every connected host terminal receives one passkey
popup naming the agent, the full prompt and the window; guests cannot answer
it. Press Tab to choose 1 hour (default), 2, 4 or 8 hours, then enter the
passkey. The critical-approval remember window never satisfies sudo. Never type
a passkey into an agent transcript, provider prompt or shell command.

The window starts at that fresh verification and covers only the work the
owner authorized. This is the prompt's durable
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

MP-08 / MP-10 / MP-11 S02: the initial owner sheet authorizes routine work
inside the task's session for the chosen window: session state, agent inventory,
session naming and directly-created agent naming/removal; public provider catalogs
and its current provider run; reading and committing/pushing changes in the
session workspace. These operations
reuse task consent, including changed parameters, with no additional popup.
Every use still checks the live task, run, creator rules and original deadline.
Owner focus remains a human terminal decision: `FocusAgent` and
`CycleAgentFocus` cannot grant resource authority to a provider, even under sudo.
Watched commands, provider command/file/tool permission gates and workspace
live-sync mutations reuse this task consent, including Required-mode runs.
The provider sandbox and live-sync filesystem fences still apply. A worker
checks its exact leased prompt against the home window on every watched-command
admission and again before launch; its cached fence alone grants nothing.
Provider permissions allow only that operation once, never a provider-wide
session grant. Startup workspace trust can reuse an already approved launch
for that exact workspace without treating the pending task as live tool authority.
Global extension registration reuses consent only for source inside this
workspace; outside source roots still ask, and App sandboxing stays in force.
Other session/workspace targets and unclassified host operations require a fresh owner
passkey in an operation-scope popup showing their exact parameters. That added
approval binds a digest to the original work and window. Scope is not inferred
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

Sudo never answers arbitrary interactions: `RespondToInteraction` is refused to
`chariox_kernel_request` and to every kernel-agent caller, and the legacy
`chariox.meta.resolve_runtime_interaction` tool is removed. Covered provider tool permission gates consume task consent before registering
an interaction. Owners answer new scope and other approvals in their terminal. Sudo also cannot mint access or sudo, answer
credential-entry prompts, export secret values, change the passkey or
configure kernel access, pairing, relay or Cloud identity.

`chariox access list` prints a JSON object with `grants` and `sudo_turns`
arrays (each sudo entry is a window). `kernel_access.sudo` records requested,
authorized, started, timer_armed, warning, extension_requested, extended and the
end reason (expired, work_ended, explicit_revoke, restart_dropped, …). Receipts
contain no passkey.

Provider discovery advertises `chariox_kernel_request` before the first turn so
official harnesses can cache its interface; each call still needs live authority.
Its `request` argument is one serialized
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
passkey. TCP, relay, ungranted peers are refused, and the window ends when the requester's grant ends. Shell CLI
calls use the tracked provider's OS process tree. For a spawn launcher, the
kernel also records the unique OS-verified endpoint server in that tree before
prompt dispatch. Only descendants born after the current elevated turn binds
are admitted below that server; other pre-turn children receive no exemption.
A descendant retained from an
earlier turn has no authority in a later turn. Sudo Unix terminal subscriptions are
limited to the window session and owner attachments; ordinary external grants
can subscribe across their local kernel. Leased sudo execution
remains a separate leg.
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

## Vault generation and protected login (A06)

MP-08 / MP-10 / MP-11 A06. During a live window the agent also sees
`chariox.vault.generate`: it supplies a `request_id`, the site `origin`
(`https`, or `http` on loopback) and an optional description, length (16-128,
default 24) and symbol choice. The kernel CSPRNG creates a password and writes
it straight into the owner's Vault, bound to that site's `host[:port]` and to
browser input only. The result is `{credential_id, origin, created}`; the value
never enters the tool call, result, transcript, events or logs. Retrying the same
`request_id` returns the same committed handle (`created: false`), never a new
value, and cannot rebind it to another site. Ordinary agents cannot generate
credentials; the former `chariox.create_generated_credential` tool is removed.

To log in, the elevated agent uses `chariox.kernel_browser_paste_secret` on a
tab of its retained browser grant without needing focus, then clicks the
observed submit button with ordinary input. The kernel fills the field; the
model never types a password. Before inserting, the kernel rechecks after
the Vault unlock wait: the same window is still live, the target document and
password field are unchanged, the credential is allowed for the document's
host, a generated handle belongs to this session, and the observation
protection did not change meanwhile. An elevated fill also requires a
credential bound to its site. Ending the window withdraws a pending unlock
prompt. Focused (human-approved) fills keep working without sudo.

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

## MP-08 / MP-10 / MP-11: remembered proof across clients

The kernel owns the critical-approval remember window for an owner. A new web
client or TUI may approve an ordinary critical decision without retyping a
passkey while that window is live. The client sends the owner's chosen answer
without proof first. Only a kernel `PASSKEY_REQUIRED` response opens trusted
passkey entry. An empty field in an already-open ordinary approval popup also
asks the kernel to check its window; no client-local timer decides permission.
New sudo windows, access grants and lifetime extensions still need fresh proof.
Remembering proof never chooses an action or grants a new sudo task.

Required live drill `kernel-access-meta/KA-REMEMBER-WEB-TUI`: use a lane-owned
identity, the real hosted web and TUI, and an ordinary critical action. Open a
five-minute remember window in web, approve another action in a fresh TUI with
no extra passkey entry, then repeat with TUI opening the window and a fresh web
client approving. Cover web DPR1 and DPR2. Verify that expiry or passkey rotation
makes the next proof-free answer require trusted entry and leaves the decision
pending. Verify fresh sudo and access/extension decisions still need proof.
Record baseline failure, fixed behavior, recordings and exact source identities.
Source and component tests alone do not accept this row.

## MP-08 / MP-10 / MP-11: legacy sudo disclosure

Required scenario `kernel-sudo/LEGACY-DISCLOSURE`: before approval, a browser
that consumes only the popup title and message must still display the target
agent, session and complete task. The message is a compatibility fallback;
structured clients also receive `sudo_request`. The full task must survive
beyond the structured summary's 160-character limit. Verify both DPR1 and DPR2
through the hosted relay, refuse the request and confirm no task runs. The
baseline must show the missing disclosure with approval still enabled.

## MP-08 / MP-10 / MP-11: task consent drill (S02)

Required live scenario `kernel-sudo/S02`: use a lane-created identity, the real
built web and TUI, hosted WSS and an official provider. Approve one `/sudo`
task to rename its own session three times. All three exact typed operations,
including their different alias parameters, must complete with zero additional
permission prompts. A harmless rename of another session must still open scope
consent and refusal must leave that session unchanged. Capture both clients at
DPR1 and DPR2 and record source/binary identities, the resolved provider model,
all consent counts and cleanup. The baseline must show the repeated prompt.
This session-resource drill does not establish unclassified host-operation,
provider-native tool approval, managed-worker, or full MP-11 acceptance.

MP-08 / MP-10 / MP-11 S02 room-admission regression: run the session-resource
scenario with room agent tools enabled. The consent sheet must not promise
provider-controlled focus. Through the authenticated `chariox_kernel_request`
path, both `FocusAgent` and `CycleAgentFocus` must fail promptly without a scope
popup or a focus change; all three covered session renames must still succeed.

## MP-08 / MP-10 / MP-11: task operation acceptance (S02, S08–S18)

Each row needs a fail-first drill against the base and the final built kernel,
real clients and official provider harness. Source regressions supplement it.

- S02: one approved task, three changed workspace-query parameters and provider
  metadata operations; no added scope sheet. A different workspace/session asks.
- S08: Required-mode watched command runs under the approved task without
  changing permanent permissions; no-sudo still refuses or asks.
- S09: Claude and native harness watched commands reuse task consent, while
  process ownership, argv/cwd and current-prompt checks remain enforced.
- S10: the same watched command on a leased worker asks no extra permission;
  revoke, expiry, replacement prompt/lease and lost home proof prevent launch.
- S11: Codex v2 and legacy command/file approval replies accept once, with no
  second interaction, only for the same task origin. Read-only denial remains.
- S13: OpenCode permission reuses consent; unfenced live-sync writes still deny.
- S14: Claude print-mode permission_prompt reuses consent for the same task.
- S15: Claude native hook and rendered permission selectors do the same.
- S16: approved exact-workspace startup trust reuses consent; a pending,
  unapproved sudo request or a newly selected workspace still asks.
- S17: Required-mode live-sync mutations reuse consent; identity, path and
  filesystem fences remain. Explicit denial and an ended task grant nothing.
- S18: workspace-sourced extension registration reuses consent; another source
  root or unspecified inline source still needs the ordinary registration sheet.

For each of Codex gpt-6-luna low, OpenCode opencode/glm-5.3-flash and Claude
claude-haiku-5-5, preapprove one task with /sudo, run a watched command and a
native provider tool approval, and require zero further prompts and real
outputs. A control without /sudo must still ask. Record the resolved model,
turn count, screenshots/video at DPR1/DPR2, prompt count and exact source/binary
identities. No provider-wide approval cache or automatic interaction-answer API
is introduced. Missing actual passwords, passkeys/codes, first account linking,
identity choice, new scope, takeover, expiry/revoke and once-only effects keep
their normal boundaries.
