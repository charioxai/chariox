# Chariox Kernel Access Plan

**Status:** Draft, 2026-09-30, revised 2026-10-06. MP-08 / MP-10 / MP-11: owner correction grants the whole local kernel; no access levels or session scope. Owner decisions of 2026-09-30 and 2026-10-01 recorded; no lead proposals remain open. This is an independent milestone, not part of Chariox Apps Phase 2.

## Summary

Today, on a laptop kernel, any process running as the owner's OS user can connect to the local kernel websocket and act as the owner. That includes an agent's own shell tool. Phase 1 of Chariox Apps closes the most dangerous consequence of this: approving a critical action now requires the Chariox passkey at approval time. The passkey is the vault passphrase.

This plan covers the rest: who may talk to the kernel, and with what authority. Access comes in strata:

1. **Chariox terminals** act for the human.
2. **External agents**, which Chariox did not launch, get the user’s ordinary authority across the local kernel on their own machine. The user grants it to the agent's process and can extend it before it expires.
3. **Sudoagents** replace today's metaagent. `/sudo <prompt>` gives a Chariox agent the user's authority on its kernel, including critical approvals, for exactly one turn.
4. **A root agent** that reaches all of the user's kernels is future work, managed by Cloud and out of scope here. The design leaves room for it.

Every grant, extension and `/sudo` needs the passkey, typed only into a kernel-owned popup that appears on every connected Chariox terminal. No bearer tokens are handed out: a sudo turn's authority stays in the kernel, and an external agent's grant is bound to its process as the OS identifies it, so there is nothing to copy. Chariox does not isolate local agents; the passkey is the hard boundary for everything dangerous.

## Owner decisions of 2026-10-01

- **D1. Passkey popups on every terminal.** Every passkey prompt (critical approvals, `/sudo`, access grants, extensions) appears as a popup on every connected Chariox terminal: TUI, web, remote TUI, and native clients later. In the TUI it is shaped like the hot-keys popup. Once the passkey is entered in one terminal, the kernel signals every terminal to close it.
- **D2. `/sudo` lasts exactly one turn.** Sudo ends when the turn yields, and the next `/sudo` needs the passkey again. There is no `/sudo off`, no sudo window, and no time expiry during the turn. The user can still interrupt the turn, which ends it.
- **D3. What a sudoagent may do.** It may answer critical approvals, including payments: "if the user gives sudo, it should be able to run potentially whatever, even payments". It may use the vault and create entries through kernel-owned flows, as agents do today. It still may not mint grants, read secret values, or change the passphrase.
- **D4. No inheritance.** Agents spawned by a sudoagent or an external agent are normal agents.
- **D5. Extension.** An external agent's access can be extended before it expires, so a legitimate agent is not cut short.
- **D6. Access strata.** (1) Chariox terminals: the human. (2) External agents: only the local kernel on the same machine, with access to every session and ordinary operation of that local kernel granted by the user (owner correction, 2026-10-06). This is the Chariox server endpoint that external agents connect to today, where they can act as the user. (3) Sudoagents: one turn. (4) Future and out of scope: the Chariox assistant, or root agent, with access to the whole Chariox system and all of the user's kernels across machines, which it discovers through Chariox Cloud. Chariox terminals will later gain the same Cloud-based kernel discovery. The design must leave room for this stratum.
- **D7. Retire the agent-terminal MCP.** External agents use only the access path of this plan: a grant through the popup. The external agent-terminal MCP of PR #15, which was never merged or tested, is superseded; #15 is closed and its branch deleted (section 9.2, PR 0).
- **D8. External agents may request `/sudo`** (lead proposal P1, confirmed). The request appears as the terminal popup, and the passkey is typed only there (section 7.1).
- **D9. No tokens; external agent access is per process** (P3, confirmed): "we don't do the token thing and instead once an external agent tries to connect, we show passphrase, and allow that PID to connect for as long as access has been granted". The kernel authorizes the requesting process, as the OS identifies it, for the whole local kernel until expiry (extendable), revocation or process exit. Its OS descendants count as the same agent; Chariox agents it spawns do not (D4). This needs the Unix socket endpoint, which is part of the core external-agent step (section 6).
- **D10. Cloud manages everything system-wide** (P4, confirmed as the direction): "Anything system-wide, including all kernels, should be managed by Cloud." The root agent's authorization builds on the Phase 2 P2.13 account-level authenticator (section 5.1).
- **D11. The passkey is the hard boundary** (the presence-check question, answered with option 1). Same-user agents can read local files and could pose as a terminal. That is accepted; the passkey guards everything dangerous.
- **D12. No isolation.** "No isolation. Chariox cannot enforce what a user installs in their machine. Isolation breaks many principles and features of Chariox." Read isolation and separate OS users for agents are not pursued. The Unix socket and process identity identify callers; they do not isolate them.
- **D13. A separate track.** The popup ships with this milestone, not in Apps Phase 1. This is a separate feature track with its own plan and milestones, and it must never delay Chariox Apps.
- **D14. No sudo safety cap.** A sudo turn has the user's authority on its kernel: no limit on critical approvals or payment amounts, and no confinement to its own session. It still may not mint grants or sudo, read secret values, or change the passphrase.
- **D15. Shared sessions** (former lead proposal P5). Only the local kernel’s owner can grant external access with that kernel’s passkey. The popup appears only on the owner’s terminals. Remote guests can never grant. A shared session does not narrow or extend the local grant. This matches Phase 1, where only the host answers approvals. The exception is the future Chariox assistant (stratum 4): it collaborates for its user, so it acts with its user's guest rights in sessions where that user is a guest (section 5.1).
- **D16. Who answers critical approvals** (former lead proposal P2): "yes, external agents can't. sudo agents can". An external agent's grant never answers critical approvals; the agent can request a `/sudo` turn instead (D8). Sudo turns can. The receipt names the sudo turn and the human `/sudo` entry that authorized it (section 7.2).

## 1. Problem and current state

### 1.1 The open local socket

The kernel listens on `ws://127.0.0.1:<port>/kernel`. `handle_kernel_connection` in `apps/kernel/src/runtime_transport.rs` checks an `Authorization` header only when `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN` or `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE` is set. Only managed and hosted workers set them. On the owner's laptop kernel no token is configured, so every loopback connection is accepted.

Once connected, a local caller with no user id is treated as the owner (`local`). `is_terminal_caller` in `apps/kernel/src/runtime/command/caller.rs` returns true for a `LocalCli` or `LocalIpc` source with a `LocalClient` caller kind and no metaagent id. `apps/kernel/src/runtime/session_membership.rs` then grants that caller the owner's view of every session. Answering a pending decision checks only that the answering user owns the operation (`apps/kernel/src/runtime/state/runtime_interaction_owned_state.rs`).

When the kernel launches a provider, it scrubs the kernel host, port and token variables from the agent's environment (`apps/kernel/src/app/provider_launch_policy.rs`, `apps/kernel/src/provider/managed_isolation.rs`). This hides the address. It does not stop a same-user process from finding the port and connecting. Read isolation for agents applies only under `CHARIOX_MANAGED_PROVIDER_ISOLATION` (managed workers). On a local kernel the macOS Seatbelt profile is a write fence around the workspace, not a read restriction.

Paths that already cannot act as the owner:

- The runtime MCP listener (`apps/kernel/src/transport/mcp_server.rs`) authenticates each provider run with its own bearer token, checks `Origin`, and only runs tools.
- The Meta tool refuses kernel-operation decisions.
- Relayed answers from a remote kernel, and answers from another user, are refused (tests in `apps/kernel/src/runtime/router/tests/session_actor_projection/kernel_operation_interactions.rs`).

The analysis and options A to D are recorded in `/Users/miguel/.codex/evidence/chariox-apps-phase1/approval-trust-boundary/README.md`.

### 1.2 Browser pages: the Origin refusal (#618)

PR #618 (open at the time of writing) makes the kernel websocket refuse any upgrade that carries an `Origin` header, with a 403, whether or not a local token is configured. Browsers always send `Origin`. Kernel clients do not: the CLI and TUI, Node and Bun websocket clients, and the web server. The browser web client reaches the kernel only through the relay. The change stops web pages open in the owner's browser from driving the kernel over loopback. It does nothing about same-user processes, which can simply omit the header.

### 1.3 Phase 1: the passkey for critical approvals

Phase 1 (protocol 392, implemented in a separate PR) requires the Chariox passkey to approve a critical action: P1.15 human validation, kernel-owned critical effects, and the App validation approval raised by `apps/kernel/src/runtime/state/app_validation_pump_runtime.rs`.

| Aspect | Phase 1 behavior |
|---|---|
| Secret | The Chariox passkey, which is the vault passphrase. No new secret. |
| Protocol | The approve choice on the projected `RuntimeInteractionChoice` carries `requires_passkey: true`. `RespondToInteraction` gains an optional `passkey` (redacted, zeroized, never logged or persisted) and an optional `passkey_remember_minutes` (1 to 15). |
| Verification | The kernel derives the key with Argon2id and compares it with a pinned commitment (the vault's KDF parameters and a hash of the derived key), kept durably. The pin is taken from the vault the boot configuration names, when the kernel first unlocks it or first sees a passkey that opens it, and is never replaced: a later change of the configured vault path or of the vault file cannot redirect verification. The vault unlock state does not change. A same-user process that can write the kernel's files before the first pin could still plant a vault; Chariox does not isolate local agents (D12), so this stays a residual risk. |
| Rate limit | 5 failures, then an exponential lockout capped at 15 minutes. |
| Remember window | Optional, off by default, up to 15 minutes, per owner, kernel memory only. Separate from the vault unlock window, because the vault is usually unlocked and its unlock does not prove a human is present. While it is open, any caller the kernel accepts as the owner (on a laptop, any same-user local process) can approve a critical action without the passkey; that is the accepted cost of opting in. |
| Deny | Needs no passkey. |
| Routine approvals | Unchanged. Tool permissions and App binding approvals in Ask mode can still be answered by the user or a metaagent without the passkey. |
| Agents | Metaagent and MCP paths stay refused for kernel-operation decisions. |
| Audit | Each passkey decision appends a durable `critical_approval.passkey` event with outcome `verified`, `remembered`, `rejected`, `missing`, `rate_limited` or `unavailable`. The passkey is never recorded. |
| Clients | The TUI approval panel and the web approval dialog ask for the passkey with hidden input, for critical approvals only. (The web App panel's approval strip shows only agent interactions, which may never require the passkey.) To a remote kernel the passkey travels end-to-end encrypted through the relay like any other kernel request; the relay sees ciphertext only. |
| Other vault backends | If the vault backend is not the encrypted Chariox vault, critical approvals fail closed. |

### 1.4 What is still open after Phase 1

Phase 1 protects critical approvals. Everything else a same-user process can do over the local socket is unchanged: prompt and spawn agents, answer routine approvals, run workflows, install and bind Apps, read every session's transcript and events. There is also no way to give an agent more than delegation-only authority without handing it the whole kernel. This milestone addresses both.

## 2. Relationship to Phase 1

- This is a separate feature track with its own plan and milestones, and it must never delay Chariox Apps (D13).
- Phase 1 ships first and does not depend on this milestone. Its contract holds until this milestone ships: agents cannot approve critical actions.
- This milestone reuses the Phase 1 pieces: the pinned passkey verifier, the rate limiter, the redaction and zeroization of the `passkey` field, the hidden-input prompts in the TUI and web, and the durable audit stream. The prompts become the popups of section 5.2 when this milestone ships; Apps Phase 1 keeps its approval panel (D13).
- An external agent's grant never satisfies a critical approval (D16). A sudo turn may answer one (D3), because the human entered the passkey for that turn.
- The Phase 1 remember window stays a Phase 1 feature for approvals at a terminal. It never satisfies a `/sudo`, a grant or an extension.

## 3. Goals and non-goals

### Goals

- Every connection to the kernel identifies itself, and the kernel knows its stratum: a Chariox terminal, an external agent, or an agent the kernel launched.
- An external agent can get ordinary authority across the local kernel, and a Chariox agent can get sudo for one turn, only after the user enters the passkey in a Chariox terminal.
- A grant works only for the process it was granted to, that process's OS descendants, on the local kernel. There is no string to copy.
- Grants default to eight hours, are bounded at 24 hours, extendable, revocable, held only in kernel memory, and audited.
- The metaagent becomes a sudoagent: normal agent behavior plus the user's authority for one `/sudo` turn.
- Each step ships on its own, with no flag day for existing clients.

### Non-goals

- Replacing the Phase 1 passkey check for critical approvals before this milestone ships.
- Isolating local agents, whether by read isolation (option D in the trust-boundary analysis) or by running agents as separate OS users. Owner decision D12: Chariox cannot enforce what a user installs on their machine, and isolation breaks many principles and features of Chariox.
- Protecting against a process that can read the kernel's memory or log keystrokes, or against an authorized agent that deliberately proxies for another process. See section 10.4.
- The root agent (stratum 4) and Cloud-based kernel discovery for terminals. Cloud manages both (D10), and the design leaves room for them (section 5.1).
- Multi-user kernels with distinct OS accounts per Chariox user.
- A new secret. The passkey stays the vault passphrase.
- Changing how the Cloud authenticates web users or how the relay encrypts traffic.
- Tool discovery for external agents, which the operation registry of #15 offered. If it is needed later, it is designed fresh on top of the grant path; it is not part of this milestone unless the owner asks.

## 4. The Chariox passkey

### 4.1 Why reuse the vault passphrase

- **One secret to remember.** Users already choose and type the vault passphrase. A second secret would be weaker in practice, because users would pick something short or reuse it anyway.
- **The kernel can already verify it.** Phase 1's `VaultPasskeyVerifier` derives a key with the pinned Argon2id parameters and compares its hash with the durable commitment. This milestone reuses that verifier. Authentication of the encrypted vault file is used only to establish the initial pin through the controlled bootstrap in section 1.3.
- **It is never given to agents.** The M26 vault plan (`docs/M26_CHARIOX_ENCRYPTED_VAULT_PLAN.md`) already guarantees that the passphrase is captured by a kernel-owned prompt and never delivered to an agent.
- **Chariox-wide.** One secret for every passkey prompt on a kernel, whatever the terminal: TUI, web or remote TUI. It belongs to the kernel's vault, so a user with several kernels has one passkey per kernel unless they choose the same passphrase for each.

### 4.2 Verification without unlocking

Every passkey check in this milestone, including access grants, extensions and `/sudo` entry, uses the same durable pinned verifier as Phase 1. Verification derives the key using the pin's salt and KDF parameters and compares its hash with the pinned commitment. Once the pin exists, verification does not read the current vault file or accept a changed configured path as a replacement authority. Vault-file authentication is confined to the controlled initial pin bootstrap described in section 1.3. A missing pin must not enable a separate bootstrap path for grants or `/sudo`.

Verification must not touch the unlocked-vault map or change any unlock expiry. Unlock policies (`operation`, `ttl`, `kernel_init`, `always`, see `apps/kernel/src/runtime/state/runtime_vault_unlock_state.rs`) are independent of verification. A verified passkey neither unlocks a locked vault nor extends an unlocked one.

Argon2id is deliberately slow and memory-hungry. Verification runs on a blocking worker, and the kernel allows one verification at a time so that a flood of attempts cannot exhaust memory. Callers that queue behind it count toward the rate limit.

### 4.3 Rate limits

The Phase 1 limiter applies to every passkey check, including the new ones in this plan (access grants, extensions, `/sudo` entry): 5 failures, then an exponential lockout capped at 15 minutes. It is per owner and held in kernel memory, so a kernel restart clears it.

A same-user process can deliberately send wrong passkeys to lock the owner out for up to 15 minutes. That is a denial of service, not a bypass, and the cap bounds it. Counting per connection class would not help: under D11 an agent can present the terminal token, so classes cannot separate its guesses from the human's (section 9.1).

### 4.4 Rotation

This milestone must add an explicit authenticated rotation operation before it can change an established pin. The operation verifies the existing passkey against the current pinned verifier, then updates the encrypted vault and the durable pin to the new passphrase through a crash-safe transition. An ordinary vault-file replacement, configuration change, or successful unlock of a different vault never rotates the pin. #700 implements this operation and ends remember windows; PRs 5 and 8 add the grant and sudo revocations below (section 9.2).

As part of a successful rotation, the kernel:

1. revokes every external agent grant,
2. ends every Phase 1 remember window,
3. cancels every queued sudo authorization and interrupts every running sudo turn, and
4. appends an audit event for each revocation, plus one for the rotation.

### 4.5 Non-encrypted vault backends

If the vault backend is not the encrypted Chariox vault, there is nothing to verify the passkey against. Critical approvals already fail closed in Phase 1. This milestone does the same: no access grants and no `/sudo`. The user sees a clear message pointing to the encrypted vault setup. Whether to offer a standalone passkey for other backends is open question 4.

### 4.6 Remote and hosted kernels

The passkey belongs to the vault of the kernel that verifies it. For a remote or hosted kernel, the user types it in the popup on the web or remote TUI. It travels end-to-end encrypted through the relay (`apps/kernel/src/transport/relay_crypto.rs`) like any other kernel request, so the relay and the Cloud see only ciphertext and never store it. Kernel-to-kernel relay peers never carry a passkey on a user's behalf.

## 5. Access strata and grants

### 5.1 The strata (D6)

| Stratum | Who | Reaches | Authority | Granted by | Ends |
|---|---|---|---|---|---|
| 1. Chariox terminals | The human, at the TUI, web, remote TUI, and native clients later | Its kernel, locally or through the relay | The owner's. Passkey popups appear here (section 5.2). | The step 1 local token (#701), or the relay (section 9.1) | Disconnect |
| 2. External agents | An agent Chariox did not launch, such as a provider CLI in the user's own shell | Only the local kernel on the same machine, through its Unix socket (D9) | The user’s on the whole local kernel for ordinary requests, without critical approvals (section 5.4) | The passkey popup (section 5.3) | Expiry unless extended, revocation, or process exit |
| 3. Sudoagents | A Chariox agent running one `/sudo` turn | Its kernel | The user's on its kernel, including critical approvals, with no cap (D14) | `/sudo` and the passkey popup | The turn yields or is interrupted |
| 4. Root agent (future) | The Chariox assistant | All of the user's kernels, discovered through Chariox Cloud | The whole Chariox system | Cloud, building on the P2.13 account-level authenticator (D10) | Future design |

Sections 1–4 preserve the original pre-implementation review. Strata 2 and 3 now have explicit admission; this revision makes stratum 2 whole-local-kernel authority while retaining fresh human confirmation for each sudo request.

**Room for stratum 4.** Every grant and sudo record carries its stratum and how the human authenticated it, and every audit event names both. Strata 1 to 3 are defined per kernel, and nothing in them assumes a user has only one kernel. Stratum 2's same-machine rule belongs to stratum 2, not to the kernel's authorization model. Cloud-based kernel discovery for terminals changes how a terminal finds a kernel, not how it authenticates to one. The design must also let the root agent operate sessions where its user is a guest, with that user's guest rights (D15); how it is authorized there belongs to the stratum 4 design.

Under D10, anything system-wide, including all kernels, is managed by Cloud. The root agent's authority across kernels cannot rest on per-kernel vault passphrases, which differ per kernel (section 4.1); it builds on the account-level authenticator of Apps Phase 2 P2.13, a Chariox-wide passkey. Its design is future work. Whether these system-wide features are also available to users who run a self-hosted relay instead of Chariox Cloud is open question 6.

### 5.2 Passkey popups (D1)

Every passkey prompt in this plan is a kernel-owned pending interaction: critical approvals, `/sudo`, access grants and extensions. The kernel projects it as a popup to every Chariox terminal connected as the owner, whether or not the terminal is attached to a session. For a shared session, only the host's terminals show it (D15). In the TUI the popup is shaped like the hot-keys popup (`apps/cli/src/hotkey-help.ts`); the web and remote TUI show the equivalent dialog.

- The first terminal where the user enters the correct passkey, or refuses, resolves the interaction, and the kernel signals every terminal to close the popup. A later answer from another terminal gets "already answered".
- A wrong passkey counts toward the rate limit (section 4.3) and leaves the popup open.
- If no terminal is connected, the interaction stays pending until it expires (`request_timeout_minutes`, section 5.5), then fails with a clear message to whoever raised it.

The popup shows what the kernel itself established, not what the requester claims: the action, the requester's stratum and identity (the OS-reported executable and process id), the target agent and session and full prompt for `/sudo`, or the whole-local-kernel scope and lifetime for a grant. Text the requester supplies, such as a reason, is labeled as such. The passkey is typed only into these popups, never into an agent's input or a CLI prompt. It travels only from the terminal to the kernel, so agents never see it.

### 5.3 Granting access to an external agent (D8, D9)

1. The external agent connects to the kernel’s local Unix socket and requests access to that local kernel. The request contains no session or access level.
2. The kernel identifies the requesting process through the OS (section 6.1) and raises the passkey popup, naming that process (executable and pid), the local-kernel boundary and proposed lifetime.
3. When the user enters the passkey, the kernel authorizes that process and its OS descendants for every ordinary local-kernel operation (section 6.2). No session selection is required, including when the kernel has no sessions yet.
4. The agent learns only the outcome. No token is issued, so there is nothing it could pass to another process or kernel.

Five minutes before a grant expires, the kernel shows the popup "access for <agent> expires in 5 minutes, extend?" (D5). Extending needs the passkey and starts a new term. If nobody extends it, the grant expires.

Only the local Unix socket carries grants. Over TCP loopback an access request is answered with a pointer to the Unix socket (section 6.3). Requests through the relay, from kernel peers, or from Apps are refused. A requester can have one pending request at a time.

### 5.4 Scope and authority

Each grant, and each queued or running sudo turn, is a kernel-side record:

| Field | Meaning |
|---|---|
| `stratum` | 2 (external agent) or 3 (sudo turn). |
| `session_id` | Sudo turn only: the agent’s session, for attribution. Access requests and grants contain no session field. |
| `holder` | Stratum 2: the process identity, which covers its OS descendants. Stratum 3: the agent, its provider run and the turn. |
| `authorized_by` | The terminal and user id that entered the passkey, the passkey audit event, and for a requested `/sudo`, the requesting external agent. |
| `expires_at` | Stratum 2 only. A sudo turn has no time expiry. |

An external agent acts as the user on its LOCAL kernel: list/create/attach all sessions, prompt and spawn agents, run and schedule workflows, install and bind Apps, answer routine approvals, and use the Vault through kernel-owned flows. No access levels or per-session scope exist. A local kernel’s internal leased-worker execution remains its own business; the holder may not attach to another kernel or send kernel-peer requests.

An external holder cannot answer critical or passkey-required approvals (including payments), mint or extend grants, change the passkey/access policy, or read/export secret values. Credential handles and kernel-owned credential creation/use remain available. It may REQUEST `/sudo` for a local agent in any local session; every request raises the same terminal popup and requires a fresh user passkey. No remember window applies, and the external holder itself never gains sudo authority.

A sudo turn acts as the user on its whole kernel, including critical approvals and payments (D14), but cannot mint grants or further sudo, read/export secret values, or change the passkey/access configuration. Agents never submit the passkey.

Agents that a sudo turn or an external agent spawns through the kernel are normal agents (D4). The spawn path never copies a holder's authority.

### 5.5 Lifetime

```toml
[kernel_access]
grant_default_minutes = 480
grant_max_minutes = 1440
grant_extend_notice_minutes = 5
request_timeout_minutes = 10
```

The user picks a grant's lifetime in the popup, up to `grant_max_minutes`. Each extension starts a new term under the same limit. A grant also ends when its bound process exits. A sudo turn has no lifetime setting: it lasts exactly one turn (D2). The owner approved these defaults on 2026-10-06; no term may exceed 24 hours.

### 5.6 Revocation

| Trigger | Notes |
|---|---|
| Session end | Cancels a queued sudo turn and interrupts a running one in it. A local-kernel access grant is independent of session lifetime. |
| Holder gone | Stratum 2: the bound process exits. Stratum 3: the turn yields, the user interrupts it, or the user cancels the queued `/sudo` prompt. |
| Explicit revoke | From any Chariox terminal, or the CLI by grant id. "Revoke all" is always available; it cancels queued sudo turns and interrupts running ones. |
| Kernel restart | Everything lives in kernel memory (section 5.7). |
| Passkey change | Section 4.4. |
| Expiry | Stratum 2 only, at `expires_at` unless extended. |

Revocation and expiry act immediately, not at the holder's next request. The kernel records which connections and subscriptions each grant or sudo turn authorized. On revocation, expiry or the end of the turn it cancels those subscriptions (the subscription loop in `runtime_transport/subscriptions.rs` runs independently of inbound requests, so an idle subscriber would otherwise keep receiving transcripts and snapshots) and closes the external agent's connections. As a second line, each delivery and replay snapshot rechecks that its grant or turn is still live. A focused test revokes and expires a grant while its client stays idle, ends a sudo turn with a subscription open, and checks that no later event or replay snapshot is delivered.

### 5.7 Storage

Grants and sudo state live only in kernel memory. Nothing is written to disk. A kernel restart revokes everything, and the user re-enters the passkey.

Nothing is delivered to the agent (D9). The 2026-09-30 draft considered handing agents a bearer token through an environment variable, a 0600 file or an inherited descriptor. Same-user processes can read each other's files and often their environment, and Chariox does not isolate them (D12), so none of these keeps a secret. A sudo turn uses the run's existing runtime MCP identity, which the kernel elevates for that turn only. That per-run MCP bearer predates this plan (`generate_runtime_mcp_auth_token` in `apps/kernel/src/app/provider_launch_policy.rs`, written into the provider's MCP configuration); section 10.4 states what it means during a sudo turn.

### 5.8 Audit

Durable events, next to `critical_approval.passkey`, never containing the passkey:

- `kernel_access.grant` with outcome `requested`, `granted`, `refused`, `extended`, `expired` or `revoked`, plus the grant id, holder, authorizing terminal and revocation reason.
- `kernel_access.sudo` with outcome `requested`, `entered`, `refused`, `ended` or `interrupted`, plus the agent, run and turn, and the requester for an external request.
- Receipts of critical approvals answered by a sudo turn name the turn and the `/sudo` entry that authorized it (D16).
- `kernel_access.denied` for each request refused for scope. These are sampled and aggregated so a looping agent cannot flood the log.

Chariox terminals and local grant holders may list the live grants and running sudo turns and revoke them. Minting and extension still require the user’s terminal passkey.

## 6. External agent binding (D9)

D9 binds access to OS process identity. There is no token, and the local kernel is the holder’s entire ordinary authority boundary (owner correction, 2026-10-06). Connections do not own grants (section 6.3).

### 6.1 Process identity

The kernel identifies the connecting process through the OS, which works only on Unix domain sockets: `getpeereid` and `LOCAL_PEERTOKEN` on macOS (the audit token avoids PID reuse), and `SO_PEERCRED` plus a pidfd or the process start time on Linux, so a reused PID cannot inherit a grant. The kernel already prepares a Unix socket for local IPC (`apps/kernel/src/local/ipc.rs`, mode 0600 in a 0700 directory), but the running kernel does not start that request/response server today. This milestone serves the kernel websocket protocol there instead and adds peer identification and a peer UID check (section 9.2, PR 5).

- **External agents.** The grant binds to the requesting process: the external agent itself. When the request comes from a helper, such as a `chariox` CLI run by the agent's shell tool, the helper names the agent process among its OS ancestors; the kernel verifies the ancestry, and the popup shows the executable and pid it will authorize. A requester that names a broad ancestor, such as a terminal emulator or a login shell, shows it in the popup. Later connections are accepted from that process and its OS descendants, checked with the same PID-reuse protection, so CLI calls from the agent's own shell tools work. Chariox agents it spawns through the kernel do not inherit (D4).
- **Kernel-launched agents.** For shell-level CLI access from a sudo turn, the kernel creates a dedicated OS session (`setsid`) at each provider launch and tracks the descendant identities it launched, with PID-reuse protection. Process-group or shared OS-session membership alone is not an identity: on Linux any process in the same session can `setpgid` into another group. A negative test has a sibling agent's process join the target's group and try to use its authority.

**Limits.** PID ancestry is fragile: daemonized helpers reparent to `launchd` or `init` and lose the grant, and sandbox wrappers or containers change the picture. A same-user process can still inject into the holder on many systems. Windows needs a different mechanism (named pipes and `GetNamedPipeClientProcessId`, open question 5).

### 6.2 Local-kernel authority (MP-08 / MP-10 / MP-11)

Every live grant authorizes ordinary requests throughout its local kernel. There is no `session_id` in the access request or grant, no filtered `ListSessions`, and no grant-specific session-scope mapping. Normal user membership and ownership checks still apply. The shared request policy refuses human-only authority changes, critical approvals, secret disclosure/export, remote-kernel attachment and peer requests. It does not reject global operations simply because they have no session.

Raw credential registry get/list, MCP registry get/list and MCP provider import responses can contain literal injection headers, environment credentials or HTTP authorization headers. External grants and sudo turns must refuse these requests before serialization; kernel-owned capability discovery remains available. A TUI must route access replies using the prompt's actual session id: only `kernel-access` prompts use the protocol-451 decision response, while older kernels retain their session-scoped interaction response. (MP-08 / MP-10 / MP-11.)

Access and extension popups use the shared kernel-owned interaction board independently of any session. Their routing identifier is not a grant scope. Closing, archiving or deleting a session cannot revoke a local-kernel grant. Each request, queued continuation, reply and subscription still checks process identity and live grant authority; expiry and revocation remain immediate.

### 6.3 TCP loopback

The kernel cannot reliably identify the process behind a TCP loopback connection: mapping it to a PID through the OS socket table is racy and platform-specific. A grant is therefore never usable over TCP. An access request there is answered with a pointer to the Unix socket, and once enforcement ships (section 9.2, PR 7) a TCP connection without the token is refused with that pointer. This is the simpler of the two options. The other, binding a grant to that one TCP connection, would add a second binding and lifecycle, and would not cover the per-command CLI, which opens a new connection each time.

## 7. The sudoagent

Today `/meta <task>` puts the focused agent into a temporary Meta mode that is delegation-only (`docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, catalog entry in `apps/kernel/src/runtime/terminal_command_catalog/catalog/core.json`). The metaagent can plan and supervise but cannot edit files or run shell tools. The owner's decision is to replace this with a sudoagent that holds the user's authority for one turn.

### 7.1 Entering

- `/sudo <prompt>` replaces `/meta <task>` on the focused agent, typed in a Chariox terminal.
- Every `/sudo` raises the passkey popup (section 5.2); the Phase 1 remember window does not apply. The prompt is sent only after the passkey is verified.
- Sudo attaches to the turn this prompt starts. If the agent is busy, the prompt waits for the current turn to yield; it is never merged into a running turn.
- While it waits, the authorization is revocable kernel-memory state. The triggers in section 5.6 cancel it, including rotation and "revoke all", and dispatch atomically checks that it is still live before the turn starts. It is never written to the durable prompt queue, so after a kernel restart the queued `/sudo` prompt is dropped with a notice instead of restored. A focused test queues `/sudo` behind a busy turn, rotates the passkey or revokes all before dispatch, lets the busy turn finish, and checks that sudo does not start without a fresh passkey.
- An agent cannot put itself or another agent into sudo.

**External `/sudo` requests (D8).** An external agent may request a `/sudo` turn for a local agent in any session on the local kernel. The request becomes a kernel-owned pending interaction, shown as the passkey popup on every connected terminal. It names the requesting agent, the target agent and session, and the full prompt. The user types the passphrase in the popup; the external agent never sees it and learns only the outcome. If no terminal is connected, the request stays pending until it expires, then fails with a clear message. Each request needs its own passphrase entry (D2).

### 7.2 What a sudo turn can do

- Everything a normal agent does: edit files, run shell and script tools, use its MCP tools.
- Act as the user on its whole kernel, with no cap (D14), through its runtime MCP tools: spawn, prompt and supervise agents in any session; create and run workflows; install and grant Apps, MCPs and skills; answer routine approvals; use the vault and create entries through kernel-owned flows.
- Answer critical approvals, including payments, with no limit on their number or amount (D3, D14).

Shell-level `chariox` CLI calls from the turn carry sudo only once process identity for kernel-launched agents exists (section 6.1). Until then, kernel access goes through the run's MCP tools.

**Critical approval receipts (D16).** The receipt of a critical approval answered by a sudo turn is attributed to the turn and to the human `/sudo` entry that authorized it. An external agent's grant (stratum 2) does not answer critical approvals; the agent can request a `/sudo` turn instead (D8). Until this milestone ships, Phase 1 is unchanged: agents cannot approve.

### 7.3 What it still cannot do

- Mint, extend or hand out grants or sudo, for itself or for other agents.
- Read secret values or export the vault.
- Change the passkey, the vault passphrase or the kernel access configuration.
- Pass sudo on: agents it spawns are normal agents (D4).

### 7.4 End and audit

- Sudo ends when the turn yields. There is no `/sudo off`, no sudo window and no time expiry during the turn: a long turn keeps sudo until it yields.
- The user can interrupt the turn as usual, which ends it. The revocation triggers in section 5.6 also interrupt it.
- The next `/sudo` needs the passkey again.
- Every transition appends a `kernel_access.sudo` event. Every kernel request made under sudo is attributed to the turn in existing traces.

### 7.5 Compatibility for existing `/meta` users

- For one release, `/meta` keeps working and shows a notice that `/sudo` replaces it. `/meta` stays delegation-only and needs no passkey, because it grants nothing new.
- Meta tasks already running (`apps/kernel/src/runtime/state/metaagent_task_runtime_state.rs`) finish in Meta mode. They are never upgraded to sudo automatically, since that would grant authority without a passkey.
- In a later release `/meta` is removed from the command catalog and the CLI (`apps/cli/src/command-center.test.ts` covers it today). Whether a delegation-only mode survives under another name is open question 2.

## 8. Effect on each client

| Client | Credential to the kernel | Passkey prompt | Change |
|---|---|---|---|
| Web via relay | Cloud session, then relay end-to-end encryption; the kernel sees a `RelayClient` with a user id | The popup (section 5.2), end-to-end encrypted to the kernel | Shows the popup for every passkey prompt, and the live grants and sudo turns with revoke. The browser never holds a grant. |
| TUI (local) | The step 1 local token (#701) | The popup, shaped like the hot-keys popup | Shows the popup, sudo state and live grants. |
| Remote TUI | Relay, like the web | The popup, end-to-end encrypted | Same as the web. |
| CLI commands | The token file when the user runs them; the external agent's grant, over the Unix socket, when run from its shell tools | Never typed in the CLI | A command that needs the passkey raises the request and waits while the user answers the popup in a terminal. With no terminal connected it fails when the request expires. |
| External agents | None of their own; the local Unix socket only | Never | Request grants and `/sudo` turns. Grants are bound to the agent's process (section 6). |
| Kernel-launched agents | The per-run runtime MCP bearer, as today | Never | A `/sudo` turn elevates it for that turn only. |
| Kernel-to-kernel relay peers (`apps/kernel/src/transport/relay_peer.rs`) | Peer identity and relay keys | Never | Grants are never forwarded, and peers cannot request them. A remote agent gets kernel access only from its own kernel. Relayed answers to kernel-operation decisions stay refused, as today. |
| Hosted and managed workers | `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`, as today, held by the host controller | Through the web, end-to-end encrypted | Unchanged. That token keeps its meaning as the host controller's credential. Grants and sudo are layered on top, and managed isolation already keeps the host token from agents. |

The same local-token pattern is used for package actions in `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md` (random kernel auth, authenticated handshake, token removed before provider launch). This plan brings it to the laptop kernel.

## 9. Migration: PR breakdown

Each PR ships on its own and leaves the system working. A PR that changes a serialized protocol shape takes the next reserved protocol number when it is implemented, under the Protocol Change Rule; this plan does not pick numbers.

### 9.1 Connection classes

From PR 2 the kernel assigns every connection one class from this fixed vocabulary, and audit events name it.

| Class | Identified by | May submit a passkey | Authority |
|---|---|---|---|
| `terminal` | The step 1 local token (#701) on TCP loopback, which every `LocalIpcClient` (TUI, CLI, native TUIs, `chariox-shell`) sends; or a relay client with a user id (web, remote TUI) | Yes, only in answer to a popup (D1) | Stratum 1 |
| `external_agent` | A Unix socket peer process, or one of its OS descendants, holding a D9 grant | No | Stratum 2 |
| `kernel_agent` | The per-run runtime MCP bearer, locally or forwarded by a worker kernel over the relay; from PR 10 also its process tree on the Unix socket | No | Its tools; sudo for one turn (stratum 3) |
| `host` | `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`, held by the host controller of managed and hosted workers | No | Unchanged |
| `relay_peer` | Peer identity and relay keys: kernel, machine and hosted-service identities | No | Unchanged; never receives grants |
| `unauthenticated` | Neither a token nor a grant | No, except on TCP until PR 7 | On TCP until PR 7: today's treatment, logged. On the Unix socket: only an access request. On TCP from PR 7: refused with a pointer to the Unix socket. |

- **The human-surface credential is the #701 token.** There is no separate file. Terminals and the CLI share it, and under D11 a same-user agent that reads it is accepted as a terminal; the passkey still guards everything dangerous.
- **Passkeys.** Only `terminal` connections may submit a passkey. A `passkey` field from any other class is refused with `PASSKEY_NOT_ACCEPTED` before verification and does not count toward the limiter.
- **No per-class lockout.** Under D11 an agent can present the terminal token, so classes cannot separate its guesses from the human's. The single per-owner limiter of Phase 1 stays (section 4.3).
- **`is_terminal_caller`** keeps its transport-source logic until PR 7, which switches it to the `terminal` class in the same change that refuses connections without the token. Clients without the token, such as iOS and drills with an isolated `CHARIOX_HOME`, are therefore never demoted before PR 6 has given them the token.

### 9.2 The PRs

0. **Retire #15 (D7). Done on 2026-10-01, with the owner's go-ahead.** PR #15 ("feat: external agent terminal MCP peer", branch `codex/agent-terminal-runtime`, one commit `95137ee73`) was never merged. It added the stdio MCP server `chariox-agent-terminal` and `chariox-shell agent-terminal`, whose `chariox_execute` tool reached every registry kernel request over the unauthenticated loopback socket with no auth of its own. It also added `GetTerminalOperationRegistry` and `TerminalOperationRegistry`, a `PromptSource` on `SubmitPrompt`, generated `contracts.json` and `parity_manifest.json`, and the docs `AGENT_TERMINAL.md` and `AGENT_TERMINAL_VALIDATION.md`. Main and `apps/p1-app-bound-copy` contained none of it, so nothing was removed from main.
   - #15 is closed with a pointer to this plan, and `codex/agent-terminal-runtime` is deleted from origin. Its head `95137ee73` is archived as a git bundle in the owner's `~/Archives`, which also keeps the commit's unrelated changes (removing `ListPromptSettings` and `GetPromptSetting`, a rewrite of `session/runtime_session/workflows.rs`, a `ci.yml` edit).
   - These stay, because other features use them: the per-run runtime MCP listener (`transport/mcp_server.rs`, `generate_runtime_mcp_auth_token`), `GetTerminalCommandCatalog`, `chariox-shell` and the `LocalIpcClient` in `@chariox/kernel-client`, and `prompt_origin: "external"`, which observes provider sessions run outside Chariox.
1. **Local token in log mode. Done in #701 (`d9926f39a`, clean review).** The laptop kernel writes `<state>/kernel-local-auth/<port>.token` (0600 in a 0700 directory, a fresh `chx_kat_`-prefixed token at each start) and accepts it on `Authorization`. Missing and wrong tokens are still admitted and logged. `LocalIpcClient` sends it; iOS and drills with an isolated `CHARIOX_HOME` do not yet (PR 6). No protocol change.
2. **Connection classes and audit attribution. Done in #705 (`7b1442043`, clean review, protocol 402 after Phase 1 renumbering).** The classes of section 9.1 are assigned at admission and named in traces and in `critical_approval.passkey`; nothing assigns `external_agent` until PR 5. A passkey from `kernel_agent`, `host`, `relay_peer` or `external_agent` gets `PASSKEY_NOT_ACCEPTED` before the check and does not count toward the lockout. `terminal` and, until PR 7, `unauthenticated` keep today's treatment, and `is_terminal_caller` is unchanged. Two judgement calls: hosted services over the relay are `relay_peer`, and relayed remote metaagents are `kernel_agent`.
3. **Passkey popups (D1, D13, D15).**
   - Scope: the kernel-owned pending interaction of section 5.2 for every passkey prompt. Critical approvals move to it, and the Phase 1 hidden-input prompts are removed in the same PR.
   - Depends on: PR 2. Protocol: yes. Clients: the TUI popup shaped like the hot-keys popup, and the web and remote TUI dialog. Native clients follow later (D1).
   - Tests: every owner terminal gets the popup, attached to the session or not; the first correct answer resolves it and every terminal closes it; a later answer gets "already answered"; a wrong passkey keeps it open and counts; with no terminal it expires with a message; a shared session shows it only on the host's terminals; deny needs no passkey.
   - Must not break: the Phase 1 contract (passkey required, remember window, audit outcomes, end-to-end encryption to remote kernels); a pasted passkey is kept exactly or refused, as #704 (`2e6f96ce3`) does for the Phase 1 field; and Apps Phase 1, which keeps its panel until this ships (D13).
4. **Passkey rotation (section 4.4). Delivered by #700 (`a3756c648`, clean review, on #632).** It adds the authenticated vault passphrase change with a crash-safe pin rotation, and ends remember windows. Revoking grants and queued or running sudo turns on rotation moves to PRs 5 and 8, which introduce them.
5. **Unix socket and external agent grants (D5, D9, D15, D16).**
   - Scope: serve the kernel websocket protocol on a Unix listener at `local_socket_path`, prepared as `apps/kernel/src/local/ipc.rs` does (0600 socket, 0700 directory). It replaces the unused request/response server there. Peer identification: `getpeereid` and `LOCAL_PEERTOKEN` on macOS; `SO_PEERCRED` plus a pidfd or the start time on Linux; other UIDs are refused. Then grants (sections 5.3 to 5.8 and 6): the request and popup, holder verification through the named ancestor, OS descendants, whole-local-kernel ordinary authority with human-only and remote boundaries, lifetime and the extension popup, revocation including process exit, idle subscriptions and passkey rotation (hooked into #700), and audit. On TCP an access request gets a pointer to the Unix socket. macOS and Linux only (open question 5).
   - Depends on: PRs 2, 3 and 4. Protocol: yes (access request, grant events, list and revoke).
   - Clients: `LocalIpcClient` gains `ws+unix://` endpoints. `chariox access request` names the holder with `--holder-pid`, which defaults to the CLI's grandparent: the agent, when the CLI runs from its shell tool. Terminals list grants with revoke and show the extension popup.
   - Tests: the popup shows the OS-reported executable and pid; a descendant is accepted, a sibling and a reused PID are refused; process exit, expiry, revoke and rotation end the grant at once, idle subscribers included; extension; cross-session/global operations work; remote-kernel requests and secret reads are refused; no session is needed to grant; a grant cannot answer a critical approval; agents a holder spawns get nothing (D4).
   - Must not break: TCP terminals with the token, tokenless clients in log mode, the MCP listener, managed workers.
6. **The token for every first-party client.**
   - Scope: drill helpers that start a kernel under a private `CHARIOX_HOME` pass that home (or `XDG_STATE_HOME`) to their clients. The iOS simulator client reads the token file named by `SIMCTL_CHILD_CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE` at each connection and sends it on loopback URLs only; if the simulator cannot read host files, it takes the value from `SIMCTL_CHILD_CHARIOX_KERNEL_LOCAL_AUTH_TOKEN` instead. Devices use the relay.
   - Depends on: PR 1. Protocol: no.
   - Tests: the drill suite in log mode logs no missing-token warning from a first-party client; an iOS unit test sends the header on loopback and never elsewhere.
   - Must not break: relay and remote clients, which never receive the token.
7. **Enforcement.**
   - Scope: `KernelLocalAuth::admit` refuses a missing or wrong local token with 401 and a body that names the token file and the Unix socket. An `unauthenticated` Unix socket connection can only request access. `is_terminal_caller` switches to the `terminal` class in this PR.
   - Depends on: PRs 2, 5 and 6, plus a log-mode run that shows no first-party client without the token. Protocol: yes. Clients: `LocalIpcClient` turns the 401 into a clear error; minimum supported versions rise where clients rely on the new behavior.
   - Tests: tokenless and wrong-token TCP upgrades get the 401 and its message; with the token everything works; `is_terminal_caller` follows the class; host-token kernels behave as before.
   - Must not break: managed and hosted workers, relay and remote clients, the MCP listener, and the publication gateway the kernel spawns.
8. **`/sudo` (section 7).**
   - Scope: `/sudo <prompt>` with the popup; the queued authorization; elevation of the run's MCP identity for one turn; kernel-wide authority (D14); critical approvals with receipts naming the turn and the `/sudo` entry (D16); external `/sudo` requests from grant holders (D8); the notice on `/meta`; revocation on rotation (hooked into #700), revoke all and session end.
   - Depends on: PRs 2, 3 and 4; PR 5 for external requests. Protocol: yes (catalog command, interaction and receipt shapes).
   - Clients: the `/sudo` entry, sudo state and running turns with revoke in the TUI and web; `chariox sudo request --agent <id>` over the Unix socket for external agents.
   - Tests: sudo ends at yield and on interrupt, with no time expiry; the queued-sudo test of section 7.1; a restart drops a queued `/sudo`; rotation cancels a queued and interrupts a running sudo turn; a critical approval answered with the receipt; a grant cannot answer one; spawned agents get no sudo; an external request's popup names the requester, target and prompt.
   - Must not break: `/meta` for one release, running Meta tasks, routine approvals.
9. **Retire `/meta`.** One release after PR 8: remove it from the catalog, the CLI and the tool set, with `apps/cli/src/command-center.test.ts` updated, and point `/meta` users to `/sudo`. Protocol: yes. The one decision left is open question 2: how long the alias lasts, and whether a delegation-only mode survives under another name.
10. **Shell-level CLI for sudo turns (section 6.1).**
    - Scope: a dedicated OS session (`setsid`) per provider launch, and kernel-tracked descendants with PID-reuse protection, so Unix socket connections from a sudo turn's process tree carry sudo.
    - Depends on: PRs 5 and 8. Protocol: no.
    - Tests: the CLI run from the turn's shell has sudo, and loses it after yield; a sibling agent's process that joins the target's process group is refused.
    - Must not break: provider launch, the Seatbelt profile, managed isolation, and environment scrubbing.

## 10. Threat model

### 10.1 Assets

- Pending decisions, above all critical approvals, including payments a sudo turn may approve.
- The passkey (the vault passphrase) and the vault contents.
- Kernel control: spawning and prompting agents, workflows, App installs and bindings.
- Session data: transcripts, events, artifacts.
- Grants, sudo turns and the audit log.

### 10.2 Actors

| Actor | Description |
|---|---|
| Same-user agent process | An agent's shell or tool, or any process it starts, running as the owner's OS user on a laptop kernel. Chariox does not isolate it (D12). The main concern of this plan. |
| External agent | An agent the user runs outside Chariox on the same machine. It may request or hold a grant, and it is also a same-user process. |
| Other OS users | Accounts on the same machine that can reach loopback. |
| Browser pages | Pages in the owner's browser that try to open a websocket to loopback. |
| Compromised relay | A relay or Cloud component that an attacker controls. |
| Remote peer | Another kernel connected through the relay. |
| Malicious App | An installed App, its backend or its workflows, trying to escalate through the kernel. |

### 10.3 Attacks and mitigations

| Attack | Mitigation |
|---|---|
| A same-user agent answers a critical approval | Phase 1: the passkey is required at approval time; agents never see it. After this milestone, only a Chariox terminal with the passkey, or a sudo turn the human started with the passkey (D3), can answer. A grant never can (D16). |
| A same-user agent acts as the owner in other ways | PRs 1 to 7 (section 9.2) require credentials and send agents through grants, which are bound to a process and limited to its local kernel’s ordinary user authority. A same-user agent that deliberately reads the terminal credential file can still pose as a terminal; that is accepted (D11), because every dangerous action needs the passkey. |
| Another OS user connects to loopback | Step 1 token in a 0600 file; the 0600 Unix socket with a peer UID check. |
| A browser page drives the kernel over loopback | #618 refuses any upgrade with an `Origin` header. |
| A compromised relay reads or replays the passkey | End-to-end encryption; the relay sees ciphertext only. Passkey fields are zeroized after use. Grants are local only and never cross the relay. |
| A remote peer answers the owner's decisions or asks for access | Relayed answers to kernel-operation decisions are already refused (#443 tests). Stratum 2 is local only, so peers cannot request grants. |
| A malicious App escalates through its workflows | App workflows never hold or request grants or sudo. App validation is a critical approval. |
| An agent reuses another agent's grant | There is no string to copy. The grant is bound to its holder's process and that process's OS descendants. No grant can cross to a remote kernel. Deliberate proxying by the holder remains (section 10.4). |
| An agent grants itself access or sudo | Every grant, extension and `/sudo` needs the passkey typed into a terminal popup. Agents can only request. |
| Request spoofing: an external request misleads the user about who is asking or for what | The popup shows what the kernel established: the requester's stratum and OS-reported process, local-kernel grant authority and lifetime, or the sudo target agent/session and full prompt. Requester-supplied text is labeled as such. |
| Request flooding: an agent fills the terminals with popups | One pending request per requester; requests expire; refusals are audited. |
| Prompt spoofing: an agent shows a fake passkey prompt in its transcript or its own UI | The passkey is typed only into kernel-owned popups with a distinct look, never inline in agent output, a CLI prompt or an external agent's UI. The docs tell users never to type it anywhere else. |
| Online guessing of the passkey | 5 failures, then exponential lockout capped at 15 minutes; one Argon2id verification at a time. |
| An agent locks the owner out with wrong guesses | A DoS bounded at 15 minutes. |
| A secret leaks into logs or transcripts | Grants and sudo issue no bearer token, and the passkey is never logged or recorded. |

### 10.4 Residual risks

- **Memory and input capture.** A same-user process that can read the kernel's memory, attach a debugger, or log keystrokes can capture the passkey. On macOS keystroke capture needs a TCC permission; under X11 on Linux any client can read keystrokes. This plan does not address it.
- **No isolation (D11, D12).** Same-user agents can read local files. They can read the step 1 token file and pose as a terminal for everything except passkey prompts, and read the vault file to guess the passkey offline, slowed only by Argon2id; passphrase strength matters more now that it gates approvals and sudo (open question 3). They can also read per-run MCP bearers from providers' MCP configurations; during a sudo turn that bearer carries sudo authority until the turn yields. This is accepted: the passkey is the hard boundary for everything dangerous, namely critical approvals, grants, extensions and `/sudo`. The local token and the Unix socket's peer UID check still stop other OS users, and well-behaved agents use the grant path.
- **Proxying.** An authorized agent can deliberately act as a proxy for another process: forward its requests over its own connections, or run it inside its own process tree. No mechanism can stop that, and the audit attributes everything to the authorized agent. The same holds for an external agent's own sub-agents and helpers, which the kernel cannot tell apart from it; D4 covers agents spawned through the kernel.
- **Sudo turn breadth.** A sudo turn can approve critical actions, including payments, in any session on its kernel for as long as it runs, and a misbehaving or prompt-injected agent can misuse that within the turn. The owner accepted this with no cap (D14). The bounds are one turn, the user's ability to interrupt it, and receipts that name the turn and the `/sudo` entry.
- **Phase 1 remember window.** Within it a human at a terminal approves without retyping the passkey, and so can a same-user agent posing as a terminal (D11); that is the accepted cost of opting in. It is off by default, lasts at most 15 minutes, and never applies to `/sudo`, grants or extensions.
- **Process identity gaps.** Processes that reparent or live in another PID namespace lose the grant (safe but inconvenient), and code injection into the holder defeats the check.

## 11. Open questions for the owner

1. **Lifetimes resolved, 2026-10-06.** Default 480 minutes, maximum 1440 minutes; retain the 5-minute extension notice and 10-minute request timeout.
2. How long should `/meta` stay as an alias, and should a no-passkey delegation-only mode survive under another name?
3. Should the kernel enforce a minimum passphrase strength now that the passphrase is the passkey, and prompt existing users with weak passphrases to rotate?
4. For non-encrypted vault backends: fail closed permanently, or offer a standalone passkey?
5. Is Windows in scope for external agent grants, which need a different process identity mechanism, or are they macOS and Linux only?
6. **Self-hosted relay** (to discuss later). Are the system-wide features managed by Cloud (D10), such as the root agent and Cloud-based kernel discovery, also available to users who run a self-hosted relay instead of Chariox Cloud?

## References

- Trust-boundary analysis and options A to D: `/Users/miguel/.codex/evidence/chariox-apps-phase1/approval-trust-boundary/README.md`
- Phase 1 passkey evidence: `/Users/miguel/.codex/evidence/chariox-apps-phase1/passkey-critical-approvals/`
- Kernel socket and local token: `apps/kernel/src/runtime_transport.rs`
- Local Unix socket IPC server: `apps/kernel/src/local/ipc.rs`
- Terminal caller check: `apps/kernel/src/runtime/command/caller.rs`
- Session membership and request scoping: `apps/kernel/src/runtime/session_membership.rs`, `apps/kernel/src/runtime/session_membership/scope.rs`
- Encrypted vault and unlock policies: `apps/kernel/src/secret/vault.rs`, `apps/kernel/src/runtime/state/runtime_vault_unlock_state.rs`, `docs/M26_CHARIOX_ENCRYPTED_VAULT_PLAN.md`
- Metaagents: `docs/M23_METAAGENTS_PLAN.md`, `docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, `apps/kernel/src/runtime/state/tool_dispatch/meta.rs`
- Runtime MCP listener and per-run bearer: `apps/kernel/src/transport/mcp_server.rs`, `apps/kernel/src/app/provider_launch_policy.rs`, `apps/kernel/src/app/provider_launch_request.rs`
- TUI hot-keys popup: `apps/cli/src/hotkey-help.ts`
- Step-up authentication (P2.13, V2-AUTH-01): `docs/CHARIOX_APPS_IMPLEMENTATION_PLAN.html`
- Package action local token: `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md`
- Superseded agent-terminal MCP: PR #15 (closed; branch `codex/agent-terminal-runtime` archived as a bundle and deleted)
