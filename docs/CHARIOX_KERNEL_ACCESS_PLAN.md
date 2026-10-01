# Chariox Kernel Access Plan

**Status:** Draft, 2026-09-30, revised 2026-10-01. Owner decisions of 2026-09-30 and 2026-10-01 recorded. The lead proposals P1 to P4 await owner confirmation. This is an independent milestone, not part of Chariox Apps Phase 2.

## Summary

Today, on a laptop kernel, any process running as the owner's OS user can connect to the local kernel websocket and act as the owner. That includes an agent's own shell tool. Phase 1 of Chariox Apps closes the most dangerous consequence of this: approving a critical action now requires the Chariox passkey at approval time. The passkey is the vault passphrase.

This plan covers the rest: who may talk to the kernel, and with what authority. Access comes in strata:

1. **Chariox terminals** act for the human.
2. **External agents**, which Chariox did not launch, get session-scoped access to the local kernel on their own machine. The user grants it and can extend it before it expires.
3. **Sudoagents** replace today's metaagent. `/sudo <prompt>` gives a Chariox agent the user's authority, including critical approvals, for exactly one turn.
4. **A root agent** that reaches all of the user's kernels is future work and out of scope. The design leaves room for it.

Every grant, extension and `/sudo` needs the passkey, typed only into a kernel-owned popup that appears on every connected Chariox terminal. The plan proposes handing out no bearer tokens: a sudo turn's authority stays in the kernel, and an external agent's grant is bound to its connection and process, so there is nothing to copy. Eventually a connection with neither a terminal credential nor a grant can only ask for access.

## Owner decisions of 2026-10-01

- **D1. Passkey popups on every terminal.** Every passkey prompt (critical approvals, `/sudo`, access grants, extensions) appears as a popup on every connected Chariox terminal: TUI, web, remote TUI, and native clients later. In the TUI it is shaped like the hot-keys popup. Once the passkey is entered in one terminal, the kernel signals every terminal to close it.
- **D2. `/sudo` lasts exactly one turn.** Sudo ends when the turn yields, and the next `/sudo` needs the passkey again. There is no `/sudo off`, no sudo window, and no time expiry during the turn. The user can still interrupt the turn, which ends it.
- **D3. What a sudoagent may do.** It may answer critical approvals, including payments: "if the user gives sudo, it should be able to run potentially whatever, even payments". It may use the vault and create entries through kernel-owned flows, as agents do today. It still may not mint grants, read secret values, or change the passphrase.
- **D4. No inheritance.** Agents spawned by a sudoagent or an external agent are normal agents.
- **D5. Extension.** An external agent's access can be extended before it expires, so a legitimate agent is not cut short.
- **D6. Access strata.** (1) Chariox terminals: the human. (2) External agents: only the local kernel on the same machine, with session-scoped access granted by the user. This is the Chariox server endpoint that external agents connect to today, where they can act as the user. (3) Sudoagents: one turn. (4) Future and out of scope: the Chariox assistant, or root agent, with access to the whole Chariox system and all of the user's kernels across machines, which it discovers through Chariox Cloud. Chariox terminals will later gain the same Cloud-based kernel discovery. The design must leave room for this stratum.

Lead proposals, each **Proposed, awaiting owner confirmation**:

- **P1.** External agents may request a `/sudo` turn, with the passkey typed only in a terminal popup. The same popup grants and extends an external agent's access (sections 5.3 and 7.1).
- **P2.** Sudo turns may answer critical approvals; an external agent's grant may not (section 7.2).
- **P3.** No bearer secrets where avoidable: sudo authority stays in the kernel, and an external agent's grant is bound to its connection and process (section 6.5).
- **P4.** The root agent's authorization likely builds on the Phase 2 P2.13 account-level authenticator. Recorded as an open question only (section 5.1).

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

Phase 1 (protocol 383, implemented in a separate PR) requires the Chariox passkey to approve a critical action: P1.15 human validation, kernel-owned critical effects, and the App validation approval raised by `apps/kernel/src/runtime/state/app_validation_pump_runtime.rs`.

| Aspect | Phase 1 behavior |
|---|---|
| Secret | The Chariox passkey, which is the vault passphrase. No new secret. |
| Protocol | The approve choice on the projected `RuntimeInteractionChoice` carries `requires_passkey: true`. `RespondToInteraction` gains an optional `passkey` (redacted, zeroized, never logged or persisted) and an optional `passkey_remember_minutes` (1 to 15). |
| Verification | The kernel derives the key with Argon2id and compares it with a pinned commitment (the vault's KDF parameters and a hash of the derived key), kept durably. The pin is taken from the vault the boot configuration names, when the kernel first unlocks it or first sees a passkey that opens it, and is never replaced: a later change of the configured vault path or of the vault file cannot redirect verification. The vault unlock state does not change. A same-user process that can write the kernel's files before the first pin could still plant a vault; that needs the file isolation of option D. |
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

- Phase 1 ships first and does not depend on this milestone. Its contract holds until this milestone ships: agents cannot approve critical actions.
- This milestone reuses the Phase 1 pieces: the pinned passkey verifier, the rate limiter, the redaction and zeroization of the `passkey` field, the hidden-input prompts in the TUI and web (which become the popups of section 5.2), and the durable audit stream.
- An external agent's grant never satisfies a critical approval. A sudo turn may answer one (D3, P2), because the human entered the passkey for that turn.
- The Phase 1 remember window stays a Phase 1 feature for approvals at a terminal. It never satisfies a `/sudo`, a grant or an extension.
- Whether the popup should replace the Phase 1 TUI approval panel already in Phase 1 is open question 3.

## 3. Goals and non-goals

### Goals

- Every connection to the kernel socket identifies itself, and the kernel knows its stratum: a Chariox terminal, an external agent, or an agent the kernel launched.
- An external agent can get session-scoped access to the local kernel, and a Chariox agent can get sudo for one turn, only after the user enters the passkey in a Chariox terminal.
- A grant works only for its holder and inside its session. There is no string to copy.
- Grants are short-lived, extendable, revocable, held only in kernel memory, and audited.
- The metaagent becomes a sudoagent: normal agent behavior plus the user's authority for one `/sudo` turn.
- Each step ships on its own, with no flag day for existing clients.

### Non-goals

- Replacing the Phase 1 passkey check for critical approvals before this milestone ships.
- Read isolation for local agents (option D in the trust-boundary analysis). This plan names where D would help but does not deliver it.
- Protecting against a process that can read the kernel's memory or log keystrokes, or against an authorized agent that deliberately proxies for another process. See section 10.4.
- The root agent (stratum 4) and Cloud-based kernel discovery for terminals. The design leaves room for both (section 5.1).
- Multi-user kernels with distinct OS accounts per Chariox user.
- A new secret. The passkey stays the vault passphrase.
- Changing how the Cloud authenticates web users or how the relay encrypts traffic.

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

A same-user process can deliberately send wrong passkeys to lock the owner out for up to 15 minutes. That is a denial of service, not a bypass, and the cap bounds it. Once terminals carry their own credential (step 2 in section 9), attempts can be counted per credential class, so failures from other connections do not lock out the TUI.

### 4.4 Rotation

This milestone must add an explicit authenticated rotation operation before it can change an established pin. The operation verifies the existing passkey against the current pinned verifier, then updates the encrypted vault and the durable pin to the new passphrase through a crash-safe transition. An ordinary vault-file replacement, configuration change, or successful unlock of a different vault never rotates the pin. Phase 1 alone does not implement this rotation operation.

As part of a successful rotation, the kernel:

1. revokes every external agent grant,
2. ends every Phase 1 remember window,
3. interrupts every running sudo turn, and
4. appends an audit event for each revocation, plus one for the rotation.

### 4.5 Non-encrypted vault backends

If the vault backend is not the encrypted Chariox vault, there is nothing to verify the passkey against. Critical approvals already fail closed in Phase 1. This milestone does the same: no access grants and no `/sudo`. The user sees a clear message pointing to the encrypted vault setup. Whether to offer a standalone passkey for other backends is open question 9.

### 4.6 Remote and hosted kernels

The passkey belongs to the vault of the kernel that verifies it. For a remote or hosted kernel, the user types it in the popup on the web or remote TUI. It travels end-to-end encrypted through the relay (`apps/kernel/src/transport/relay_crypto.rs`) like any other kernel request, so the relay and the Cloud see only ciphertext and never store it. Kernel-to-kernel relay peers never carry a passkey on a user's behalf.

## 5. Access strata and grants

### 5.1 The strata (D6)

| Stratum | Who | Reaches | Authority | Granted by | Ends |
|---|---|---|---|---|---|
| 1. Chariox terminals | The human, at the TUI, web, remote TUI, and native clients later | Its kernel, locally or through the relay | The owner's. Passkey popups appear here (section 5.2). | The human-surface credential (section 9, step 2) | Disconnect |
| 2. External agents | An agent Chariox did not launch, such as a provider CLI in the user's own shell | Only the local kernel on the same machine, through the Chariox server endpoint (the local kernel socket) | The user's within one session, without critical approvals (section 5.4) | The passkey popup (section 5.3) | Expiry unless extended, or revocation |
| 3. Sudoagents | A Chariox agent running one `/sudo` turn | Its kernel | The user's, including critical approvals (section 7) | `/sudo` and the passkey popup | The turn yields or is interrupted |
| 4. Root agent (future) | The Chariox assistant | All of the user's kernels, discovered through Chariox Cloud | The whole Chariox system | Open (P4) | Open |

Today strata 2 and 3 do not exist as such: any same-user process can act as stratum 1 over the local socket (section 1.1), and the metaagent is delegation-only.

**Room for stratum 4.** Every grant and sudo record carries its stratum and how the human authenticated it, and every audit event names both. Strata 1 to 3 are defined per kernel, and nothing in them assumes a user has only one kernel. Stratum 2's same-machine rule belongs to stratum 2, not to the kernel's authorization model. Cloud-based kernel discovery for terminals changes how a terminal finds a kernel, not how it authenticates to one.

**P4. Proposed, awaiting owner confirmation.** The root agent's authority across kernels cannot rest on per-kernel vault passphrases, which differ per kernel (section 4.1). The likely path is the account-level authenticator of Apps Phase 2 P2.13, a Chariox-wide passkey. This plan records it as open question 2 only.

### 5.2 Passkey popups (D1)

Every passkey prompt in this plan is a kernel-owned pending interaction: critical approvals, `/sudo`, access grants and extensions. The kernel projects it as a popup to every Chariox terminal connected as the owner, whether or not the terminal is attached to the session. In the TUI the popup is shaped like the hot-keys popup (`apps/cli/src/hotkey-help.ts`); the web and remote TUI show the equivalent dialog.

- The first terminal where the user enters the correct passkey, or refuses, resolves the interaction, and the kernel signals every terminal to close the popup. A later answer from another terminal gets "already answered".
- A wrong passkey counts toward the rate limit (section 4.3) and leaves the popup open.
- If no terminal is connected, the interaction stays pending until it expires (`request_timeout_minutes`, section 5.5), then fails with a clear message to whoever raised it.

The popup shows what the kernel itself established, not what the requester claims: the action, the requester's stratum and identity (on a Unix socket, the OS-reported executable and process id), the target agent and session, the full prompt for a `/sudo`, and the lifetime for a grant. Text the requester supplies, such as a reason, is labeled as such. The passkey is typed only into these popups, never into an agent's input or a CLI prompt. It travels only from the terminal to the kernel, so agents never see it.

### 5.3 Granting access to an external agent (P1, P3)

**Proposed, awaiting owner confirmation.**

1. The external agent connects to the local kernel socket and requests access to one session, with an optional reason.
2. The kernel raises the passkey popup, naming the agent, the session and the proposed lifetime.
3. When the user enters the passkey, the kernel creates the grant on the requesting connection (option (a), section 6). On a Unix socket endpoint it also binds the grant to the agent's process identity (option (c)). Every request under the grant is checked against its session (option (b)), deny by default.
4. The agent learns only the outcome. No token is issued, so there is nothing it could pass to another agent or session.

Five minutes before a grant expires, the kernel shows the popup "access for <agent> expires in 5 minutes, extend?" (D5). Extending needs the passkey and starts a new term. If nobody extends it, the grant expires.

Only connections on the local kernel socket can request a grant. Requests through the relay, from kernel peers, or from Apps are refused. A requester can have one pending request at a time.

### 5.4 Scope and authority

Each grant, and each running sudo turn, is a kernel-side record:

| Field | Meaning |
|---|---|
| `stratum` | 2 (external agent) or 3 (sudo turn). |
| `session_id` | The one session it is valid for. Required. |
| `holder` | Stratum 2: the connection, plus the process identity on a Unix socket. Stratum 3: the agent, its provider run and the turn. |
| `authorized_by` | The terminal and user id that entered the passkey, the passkey audit event, and for a requested `/sudo`, the requesting external agent. |
| `expires_at` | Stratum 2 only. A sudo turn has no time expiry. |

Both strata act as the user within their session. Requests that name another session are refused, and requests with no session scope are refused unless allowlisted for the stratum. A sudo turn's allowlist is wider (for example App and MCP installs), because the user gave that turn their authority. Neither stratum may:

- mint, extend or hand out grants or sudo, for itself or anyone else;
- read secret values or export the vault (both may use credential handles and create vault entries through kernel-owned flows, as agents do today);
- change the passkey or the kernel access configuration.

Only a sudo turn may answer critical approvals (P2, section 7.2).

Agents that a sudo turn or an external agent spawns through the kernel are normal agents (D4). The spawn path never copies a holder's authority.

### 5.5 Lifetime

```toml
[kernel_access]
grant_default_minutes = 30
grant_max_minutes = 240
grant_extend_notice_minutes = 5
request_timeout_minutes = 10
```

The user picks a grant's lifetime in the popup, up to `grant_max_minutes`. Each extension starts a new term under the same limit. Over TCP loopback a grant also ends when its connection closes; on a Unix socket it ends when the bound process exits. A sudo turn has no lifetime setting: it lasts exactly one turn (D2). The defaults are for the owner to confirm (open question 5).

### 5.6 Revocation

| Trigger | Notes |
|---|---|
| Session end | Session close, archive or delete. Interrupts a sudo turn running in it. |
| Holder gone | Stratum 2: the connection closes (TCP loopback) or the bound process exits (Unix socket). Stratum 3: the turn yields or the user interrupts it. |
| Explicit revoke | From any Chariox terminal, or the CLI by grant id. "Revoke all" is always available and interrupts running sudo turns. |
| Kernel restart | Everything lives in kernel memory (section 5.7). |
| Passkey change | Section 4.4. |
| Expiry | Stratum 2 only, at `expires_at` unless extended. |

Revocation and expiry act immediately, not at the holder's next request. The kernel records which connections and subscriptions each grant or sudo turn authorized. On revocation, expiry or the end of the turn it cancels those subscriptions (the subscription loop in `runtime_transport/subscriptions.rs` runs independently of inbound requests, so an idle subscriber would otherwise keep receiving transcripts and snapshots) and closes the external agent's connections. As a second line, each delivery and replay snapshot rechecks that its grant or turn is still live. A focused test revokes and expires a grant while its client stays idle, ends a sudo turn with a subscription open, and checks that no later event or replay snapshot is delivered.

### 5.7 Storage

Grants and sudo state live only in kernel memory. Nothing is written to disk. A kernel restart revokes everything, and the user re-enters the passkey. Grants are bound to live connections and processes, so persisting them across a restart would not help.

Nothing is delivered to the agent (P3). The 2026-09-30 draft considered handing agents a bearer token through an environment variable, a 0600 file or an inherited descriptor. Without read isolation, none of these keeps a secret from another same-user process, which is why P3 avoids bearer secrets. A sudo turn uses the run's existing runtime MCP identity, which the kernel elevates for that turn only. That per-run MCP bearer predates this plan (`generate_runtime_mcp_auth_token` in `apps/kernel/src/app/provider_launch_policy.rs`, written into the provider's MCP configuration); section 10.4 states what it means during a sudo turn.

### 5.8 Audit

Durable events, next to `critical_approval.passkey`, never containing the passkey:

- `kernel_access.grant` with outcome `requested`, `granted`, `refused`, `extended`, `expired` or `revoked`, plus the grant id, session id, holder, authorizing terminal and revocation reason.
- `kernel_access.sudo` with outcome `requested`, `entered`, `refused`, `ended` or `interrupted`, plus the agent, run and turn, and the requester for an external request.
- Receipts of critical approvals answered by a sudo turn name the turn and the `/sudo` entry that authorized it (P2).
- `kernel_access.denied` for each request refused for scope. These are sampled and aggregated so a looping agent cannot flood the log.

Chariox terminals list the live grants and running sudo turns, with a revoke control.

## 6. Binding a grant to its holder

The 2026-09-30 draft asked how to keep a token within its scope. With no token (P3), the same three options decide who holds a grant and what it reaches. They combine.

### 6.1 Option (a): connection binding

The grant belongs to the connection that requested it. There is nothing to redeem, so there is no race and no string to copy. On loopback there is no TLS, so this is connection identity rather than cryptographic channel binding.

- **Pros:** revocation is exact: close the connection. Simple to implement.
- **Cons:** it does not scope anything by itself, so (b) is still needed. The CLI opens one connection per command, so over TCP loopback each command would need its own grant unless the agent keeps one connection open.

### 6.2 Option (b): kernel-side session scoping

The kernel keeps each grant's `{stratum, session_id, holder}`. For every request it computes the request's session scope, as `request_session_scope` in `apps/kernel/src/runtime/session_membership/scope.rs` already does (`SessionId`, `SessionIds`, `SessionRef`, `AttachmentId`, `AllSessions`), and refuses anything outside the grant's session. `AllSessions` requests such as `ListSessions` are filtered to the grant's session. Requests with no session scope (global queries) are refused unless allowlisted for the stratum.

- **Pros:** it is the only option that actually enforces "this session only". It reuses existing scope machinery. It is independent of the OS, and applies equally to the local socket and the runtime MCP listener.
- **Cons:** every request type needs a correct scope mapping, and a missing or wrong mapping is a hole. Today `request_session_scope` returns `None` for global requests, so deny by default is essential. New request types add maintenance cost; a test should fail when a request type has no decided scope.

### 6.3 Option (c): process identity

The kernel identifies the connecting process through the OS: `getpeereid` and `LOCAL_PEERPID` or `LOCAL_PEERTOKEN` on macOS (the audit token avoids PID reuse), and `SO_PEERCRED` plus a pidfd or the process start time on Linux. These work only on Unix domain sockets, so this option means adding a Unix socket endpoint (mode 0600) next to, or instead of, the loopback TCP port. Mapping a TCP loopback connection back to a PID through the OS socket table is possible, but racy and platform-specific.

- **External agents.** The grant binds to the agent's own process, which the requesting CLI names among its ancestors. The kernel verifies the ancestry, and the popup shows that process's executable and pid before the user enters the passkey. Later connections are accepted only from that process or its descendants, checked with the same PID-reuse protection, so per-command CLI connections from the agent's shell keep the grant. A requester that names a broad ancestor, such as a terminal emulator or a login shell, shows it in the popup.
- **Kernel-launched agents.** For shell-level CLI access from a sudo turn, the kernel creates a dedicated OS session (`setsid`) at each provider launch and tracks the descendant identities it launched, with PID-reuse protection. Process-group or shared OS-session membership alone is not an identity: on Linux any process in the same session can `setpgid` into another group. A negative test has a sibling agent's process join the target's group and try to use its authority.

- **Pros:** a grant cannot be used from any other process. The OS user check also stops other OS users outright.
- **Cons:** PID ancestry is fragile: daemonized helpers reparent to `launchd` or `init`, and sandbox wrappers or containers change the picture. A same-user process can still inject into the holder on many systems. It does not apply to remote clients, and Windows needs a different mechanism (named pipes and `GetNamedPipeClientProcessId`). It is the most platform code of the three.

### 6.4 Comparison

| | (a) Connection | (b) Session scoping | (c) Process identity |
|---|---|---|---|
| Enforces the session scope | No, needs (b) | Yes | No, needs (b) |
| Covers a new connection from the same agent (per-command CLI) | No | Not applicable | Yes, from the same process tree |
| Lets another process use the grant | Only through the holder's connection | Not applicable | Only through the holder (proxying or injection) |
| Platform-specific code | None | None | Significant (macOS, Linux; Windows open) |
| Main failure mode | One grant per connection for the per-command CLI | A request type with a wrong scope mapping | Reparented or containerized processes rejected; injection into the holder |
| Implementation cost | Low | Medium | High |

### 6.5 Proposal (P3)

**Proposed, awaiting owner confirmation.** This answers the owner's question whether a token can be passed to other agents or sessions: hand out no bearer secrets where avoidable.

- **Sudoagent.** The authority is held kernel-side and attached to the one provider-run turn the kernel launched. No token exists.
- **External agent.** The grant binds to the requesting connection (a); on a Unix socket endpoint, also to the requesting process identity (c); and (b) scopes every request to its session, deny by default. There is no string to copy, so another agent cannot reuse the grant. Repeated per-command CLI connections from the same process tree are covered by (c). Over TCP loopback the grant falls back to the connection.
- **Residual risk.** An authorized agent can deliberately act as a proxy for another process. No mechanism can stop that, and the audit attributes everything to the authorized agent (section 10.4).

## 7. The sudoagent

Today `/meta <task>` puts the focused agent into a temporary Meta mode that is delegation-only (`docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, catalog entry in `apps/kernel/src/runtime/terminal_command_catalog/catalog/core.json`). The metaagent can plan and supervise but cannot edit files or run shell tools. The owner's decision is to replace this with a sudoagent that holds the user's authority for one turn.

### 7.1 Entering

- `/sudo <prompt>` replaces `/meta <task>` on the focused agent, typed in a Chariox terminal.
- Every `/sudo` raises the passkey popup (section 5.2); the Phase 1 remember window does not apply. The prompt is sent only after the passkey is verified.
- Sudo attaches to the turn this prompt starts. If the agent is busy, the prompt waits for the current turn to yield; it is never merged into a running turn.
- An agent cannot put itself or another agent into sudo.

**External `/sudo` requests (P1). Proposed, awaiting owner confirmation.** An external agent may request a `/sudo` turn for an agent in a session it holds a grant for. The request becomes a kernel-owned pending interaction, shown as the passkey popup on every connected terminal. It names the requesting agent, the target agent and session, and the full prompt. The user types the passphrase in the popup; the external agent never sees it and learns only the outcome. If no terminal is connected, the request stays pending until it expires, then fails with a clear message. Each request needs its own passphrase entry (D2).

### 7.2 What a sudo turn can do

- Everything a normal agent does: edit files, run shell and script tools, use its MCP tools.
- Act as the user through its runtime MCP tools, within section 5.4: spawn, prompt and supervise agents; create and run workflows; install and grant MCPs and skills; answer routine approvals; use the vault and create entries through kernel-owned flows.
- Answer critical approvals, including payments (D3).

Shell-level `chariox` CLI calls from the turn carry sudo only once the Unix socket and process identity for kernel-launched agents exist (section 6.3). Until then, kernel access goes through the run's MCP tools.

**Critical approvals (P2). Proposed, awaiting owner confirmation.** A sudo turn may answer critical approvals (D3). The receipt is attributed to the sudo turn and to the human `/sudo` entry that authorized it. An external agent's grant (stratum 2) does not answer critical approvals; the agent can request a `/sudo` turn instead (P1). Until this milestone ships, Phase 1 is unchanged: agents cannot approve.

### 7.3 What it still cannot do

- Mint, extend or hand out grants or sudo, for itself or for other agents.
- Read secret values or export the vault.
- Change the passkey, the vault passphrase or the kernel access configuration.
- Pass sudo on: agents it spawns are normal agents (D4).
- Act in another session. This confinement comes from the 2026-09-30 draft; whether to keep it is part of open question 4.

### 7.4 End and audit

- Sudo ends when the turn yields. There is no `/sudo off`, no sudo window and no time expiry during the turn: a long turn keeps sudo until it yields.
- The user can interrupt the turn as usual, which ends it. The revocation triggers in section 5.6 also interrupt it.
- The next `/sudo` needs the passkey again.
- Every transition appends a `kernel_access.sudo` event. Every kernel request made under sudo is attributed to the turn in existing traces.

### 7.5 Compatibility for existing `/meta` users

- For one release, `/meta` keeps working and shows a notice that `/sudo` replaces it. `/meta` stays delegation-only and needs no passkey, because it grants nothing new.
- Meta tasks already running (`apps/kernel/src/runtime/state/metaagent_task_runtime_state.rs`) finish in Meta mode. They are never upgraded to sudo automatically, since that would grant authority without a passkey.
- In a later release `/meta` is removed from the command catalog and the CLI (`apps/cli/src/command-center.test.ts` covers it today). Whether a delegation-only mode survives under another name is open question 7.

## 8. Effect on each client

| Client | Credential to the kernel | Passkey prompt | Change |
|---|---|---|---|
| Web via relay | Cloud session, then relay end-to-end encryption; the kernel sees a `RelayClient` with a user id | The popup (section 5.2), end-to-end encrypted to the kernel | Shows the popup for every passkey prompt, and the live grants and sudo turns with revoke. The browser never holds a grant. |
| TUI (local) | The local-kernel auth token (step 1), later a human-surface credential (step 2) | The popup, shaped like the hot-keys popup | Shows the popup, sudo state and live grants. |
| Remote TUI | Relay, like the web | The popup, end-to-end encrypted | Same as the web. |
| CLI commands | The token file when the user runs them; a grant when an external agent runs them | Never typed in the CLI | A command that needs the passkey raises the request and waits while the user answers the popup in a terminal. With no terminal connected it fails when the request expires. |
| External agents | None of their own; the local kernel socket only | Never | Request grants and `/sudo` turns. Grants are bound as in section 6.5. |
| Kernel-launched agents | The per-run runtime MCP bearer, as today | Never | A `/sudo` turn elevates it for that turn only. |
| Kernel-to-kernel relay peers (`apps/kernel/src/transport/relay_peer.rs`) | Peer identity and relay keys | Never | Grants are never forwarded, and peers cannot request them. A remote agent gets kernel access only from its own kernel. Relayed answers to kernel-operation decisions stay refused, as today. |
| Hosted and managed workers | `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`, as today, held by the host controller | Through the web, end-to-end encrypted | Unchanged. That token keeps its meaning as the host controller's credential. Grants and sudo are layered on top, and managed isolation already keeps the host token from agents. |

The same local-token pattern is used for package actions in `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md` (random kernel auth, authenticated handshake, token removed before provider launch). This plan brings it to the laptop kernel.

## 9. Migration steps

Each step ships on its own and leaves the system working. Steps 3 to 7 change serialized protocol shapes, so each needs a future protocol version under the Protocol Change Rule.

1. **Local-kernel auth token (option A), defense in depth.** On start, the laptop kernel writes a random token to a 0600 file in its state directory and accepts it on the handshake `Authorization` header, as managed workers already do. The TUI and CLI read the file. At first, connections without it are accepted but logged. This stops other OS users once enforced, and prepares every client.
2. **Human-surface credentials.** The kernel records the stratum of each connection (host token, Chariox terminal, external agent, kernel-launched agent), counts passkey failures per class, and attributes audit events to it. `is_terminal_caller` becomes a statement about credential class rather than transport source.
3. **Passkey popups.** The kernel-owned pending interaction of section 5.2, projected to every connected terminal and closed everywhere once resolved. Whether Phase 1's critical approvals move to it at this step or earlier is open question 3.
4. **External agent grants.** Requests, the popup, connection binding (a), session scoping (b) with deny by default, lifetime, extension, revocation and audit (sections 5 and 6). Grants are optional at first: connections without one keep working as today.
5. **Restrict the local socket.** A connection with neither a terminal credential nor a grant can only request access. Old clients get a clear upgrade message instead of a bare refusal. Managed workers are unaffected.
6. **`/sudo`.** One-turn sudo (section 7) ships alongside `/meta`, with the notice on `/meta`, critical approvals by sudo turns (P2) and external `/sudo` requests (P1).
7. **Retire `/meta`.** Remove it from the catalog, the CLI and the tool set after one release.
8. **Unix socket and process identity.** The 0600 Unix socket endpoint and option (c), for external agent grants and for shell-level CLI access from sudo turns. Until then, external agent grants fall back to connection binding. Read isolation (option D) stays optional.

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
| Same-user agent process | An agent's shell or tool, or any process it starts, running as the owner's OS user on a laptop kernel. The main concern of this plan. |
| External agent | An agent the user runs outside Chariox on the same machine. It may request or hold a grant, and it is also a same-user process. |
| Other OS users | Accounts on the same machine that can reach loopback. |
| Browser pages | Pages in the owner's browser that try to open a websocket to loopback. |
| Compromised relay | A relay or Cloud component that an attacker controls. |
| Remote peer | Another kernel connected through the relay. |
| Malicious App | An installed App, its backend or its workflows, trying to escalate through the kernel. |

### 10.3 Attacks and mitigations

| Attack | Mitigation |
|---|---|
| A same-user agent answers a critical approval | Phase 1: the passkey is required at approval time; agents never see it. After this milestone, only a Chariox terminal with the passkey, or a sudo turn the human started with the passkey (P2), can answer. A grant never can. |
| A same-user agent acts as the owner in other ways | Steps 1 to 5: credentials required; a connection without a terminal credential or a grant can only request access; grants scoped by (b) and bound by (a) and (c). |
| Another OS user connects to loopback | Step 1 token in a 0600 file; later a 0600 Unix socket with a peer UID check. |
| A browser page drives the kernel over loopback | #618 refuses any upgrade with an `Origin` header. |
| A compromised relay reads or replays the passkey | End-to-end encryption; the relay sees ciphertext only. Passkey fields are zeroized after use. Grants are local only and never cross the relay. |
| A remote peer answers the owner's decisions or asks for access | Relayed answers to kernel-operation decisions are already refused (#443 tests). Stratum 2 is local only, so peers cannot request grants. |
| A malicious App escalates through its workflows | App workflows never hold or request grants or sudo. App validation is a critical approval. |
| An agent reuses another agent's grant | There is no string to copy. The grant is bound to its holder's connection, and on a Unix socket to its process tree; (b) confines it to one session. Deliberate proxying by the holder remains (section 10.4). |
| An agent grants itself access or sudo | Every grant, extension and `/sudo` needs the passkey typed into a terminal popup. Agents can only request. |
| Request spoofing: an external request misleads the user about who is asking or for what | The popup shows what the kernel established: the requester's stratum and OS-reported process, the target agent and session, and the full prompt. Requester-supplied text is labeled as such. |
| Request flooding: an agent fills the terminals with popups | One pending request per requester; requests expire; refusals are audited. |
| Prompt spoofing: an agent shows a fake passkey prompt in its transcript or its own UI | The passkey is typed only into kernel-owned popups with a distinct look, never inline in agent output, a CLI prompt or an external agent's UI. The docs tell users never to type it anywhere else. |
| Online guessing of the passkey | 5 failures, then exponential lockout capped at 15 minutes; one Argon2id verification at a time. |
| An agent locks the owner out with wrong guesses | A DoS bounded at 15 minutes; per-class counting after step 2. |
| A secret leaks into logs or transcripts | Grants and sudo issue no bearer token, and the passkey is never logged or recorded. |

### 10.4 Residual risks

- **Memory and input capture.** A same-user process that can read the kernel's memory, attach a debugger, or log keystrokes can capture the passkey. On macOS keystroke capture needs a TCC permission; under X11 on Linux any client can read keystrokes. This plan does not address it.
- **File reads by agents.** Without read isolation (option D), local agents can read the token file from step 1, the vault file, and the per-run MCP bearers in providers' MCP configurations. The token file then only stops other OS users. The vault file enables offline guessing of the passkey, slowed only by Argon2id, so passphrase strength matters more now that it gates approvals and sudo (open question 8). During a sudo turn the run's MCP bearer carries sudo authority, so a process that reads it can use that authority until the turn yields.
- **Proxying.** An authorized agent can deliberately act as a proxy for another process: forward its requests over the granted connection, or run it inside its own process tree. No mechanism can stop that, and the audit attributes everything to the authorized agent. The same holds for an external agent's own sub-agents and helpers, which the kernel cannot tell apart from it; D4 covers agents spawned through the kernel.
- **Sudo turn breadth.** A sudo turn can approve critical actions, including payments, for as long as it runs, and a misbehaving or prompt-injected agent can misuse that within the turn. The bounds are one turn, the user's ability to interrupt it, and receipts that name the turn and the `/sudo` entry. Open question 4 asks about a safety cap.
- **Phase 1 remember window.** Within it a human at a terminal approves without retyping the passkey. It is off by default, lasts at most 15 minutes, and never applies to `/sudo`, grants or extensions.
- **Process identity gaps.** With (c), processes that reparent or live in another PID namespace are rejected (safe but inconvenient), and code injection into the holder defeats the check. Over TCP loopback a grant is bound to its connection only.

## 11. Open questions for the owner

1. **Lead proposals.** Confirm P1 (external `/sudo` requests through the terminal popup, section 7.1), P2 (critical approvals by sudo turns only, section 7.2), P3 (no bearer secrets, section 6.5) and P4 (the root-agent direction, section 5.1). P3 replaces the earlier questions on session-scoping enforcement, token delivery, token granularity, token persistence and token levels.
2. **Root agent.** How is the root agent (stratum 4) authorized across the user's kernels? Per-kernel vault passphrases do not fit; P4 points to the Phase 2 P2.13 account-level authenticator.
3. **Phase 1 approval panel.** Should the popup replace the passkey input of the Phase 1 TUI approval panel already in Phase 1, or only when this milestone ships?
4. **Sudo safety cap.** D2 gives a sudo turn no time expiry. Should it still have a safety cap, such as a limit on critical approvals or payment amounts per turn, or confinement to its own session (section 7.3)?
5. **Lifetimes.** Confirm the grant default of 30 minutes and maximum of 240, the 5-minute extension notice, and the 10-minute request timeout.
6. **Presence on laptops.** Accept that the human-surface credential file is readable by same-user agents until read isolation exists, so an agent can pose as a terminal for everything except passkey prompts? Or add an OS presence check (option C, Touch ID or polkit) to the popup?
7. How long should `/meta` stay as an alias, and should a no-passkey delegation-only mode survive under another name?
8. Should the kernel enforce a minimum passphrase strength now that the passphrase is the passkey, and prompt existing users with weak passphrases to rotate?
9. For non-encrypted vault backends: fail closed permanently, or offer a standalone passkey?
10. Is Windows in scope for the process identity check, or is (c) macOS and Linux only?
11. Should grants be allowed for sessions shared with other users? If so, whose passkey authorizes them, and whose terminals show the popup?

## References

- Trust-boundary analysis and options A to D: `/Users/miguel/.codex/evidence/chariox-apps-phase1/approval-trust-boundary/README.md`
- Phase 1 passkey evidence: `/Users/miguel/.codex/evidence/chariox-apps-phase1/passkey-critical-approvals/`
- Kernel socket and local token: `apps/kernel/src/runtime_transport.rs`
- Terminal caller check: `apps/kernel/src/runtime/command/caller.rs`
- Session membership and request scoping: `apps/kernel/src/runtime/session_membership.rs`, `apps/kernel/src/runtime/session_membership/scope.rs`
- Encrypted vault and unlock policies: `apps/kernel/src/secret/vault.rs`, `apps/kernel/src/runtime/state/runtime_vault_unlock_state.rs`, `docs/M26_CHARIOX_ENCRYPTED_VAULT_PLAN.md`
- Metaagents: `docs/M23_METAAGENTS_PLAN.md`, `docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, `apps/kernel/src/runtime/state/tool_dispatch/meta.rs`
- Runtime MCP listener and per-run bearer: `apps/kernel/src/transport/mcp_server.rs`, `apps/kernel/src/app/provider_launch_policy.rs`, `apps/kernel/src/app/provider_launch_request.rs`
- TUI hot-keys popup: `apps/cli/src/hotkey-help.ts`
- Step-up authentication (P2.13, V2-AUTH-01): `docs/CHARIOX_APPS_IMPLEMENTATION_PLAN.html`
- Package action local token: `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md`
