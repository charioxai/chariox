# Chariox Kernel Access Plan

**Status:** Draft, 2026-09-30. Owner decisions of 2026-09-30 recorded. This is an independent milestone, not part of Chariox Apps Phase 2.

## Summary

Today, on a laptop kernel, any process running as the owner's OS user can connect to the local kernel websocket and act as the owner. That includes an agent's own shell tool. Phase 1 of Chariox Apps closes the most dangerous consequence of this: approving a critical action now requires the Chariox passkey at approval time. The passkey is the vault passphrase.

This plan covers the rest: who may talk to the kernel, and with what authority. It proposes:

1. Kernel access tokens, minted per agent or per session after the user enters the passkey. An agent that holds one has user-level or metaagent-level kernel access within the token's scope.
2. A "sudoagent" that replaces today's metaagent. `/sudo` replaces `/meta`. Entering it needs the passkey or an active sudo window, and the sudoagent behaves like a normal agent instead of being delegation-only.
3. A local socket that eventually refuses unauthenticated connections.

How to enforce that a token only works for its own session is an open question. Section 6 lays out three options and a tentative recommendation for the owner to decide.

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

- Phase 1 ships first and does not depend on this milestone. Nothing here changes the Phase 1 contract for critical approvals.
- This milestone reuses the Phase 1 pieces: the passkey verifier, the rate limiter, the redaction and zeroization of the `passkey` field, the hidden-input prompts in the TUI and web, and the durable audit stream.
- A kernel access token never satisfies a critical approval. Critical approvals keep requiring a fresh passkey, or the Phase 1 remember window, on a human surface.
- The Phase 1 remember window and the sudo window proposed here are separate windows with separate purposes. Whether to merge them is open question 5.

## 3. Goals and non-goals

### Goals

- Every connection to the kernel socket identifies itself with a credential, and the kernel knows whether it is a human surface or an agent.
- An agent can be granted user-level or metaagent-level kernel access for a bounded scope and time, only after the user enters the passkey.
- A granted token works only inside its scope: its session, and when per-agent, its agent.
- Tokens are short-lived, revocable, never persisted in clear, and audited.
- The metaagent becomes a sudoagent: normal agent behavior, entered through `/sudo` with the passkey or an active sudo window.
- Each step ships on its own, with no flag day for existing clients.

### Non-goals

- Replacing the Phase 1 passkey check for critical approvals.
- Read isolation for local agents (option D in the trust-boundary analysis). This plan names where D would help but does not deliver it.
- Protecting against a process that can read the kernel's memory or log keystrokes. See section 10.4.
- Multi-user kernels with distinct OS accounts per Chariox user.
- A new secret. The passkey stays the vault passphrase.
- Changing how the Cloud authenticates web users or how the relay encrypts traffic.

## 4. The Chariox passkey

### 4.1 Why reuse the vault passphrase

- **One secret to remember.** Users already choose and type the vault passphrase. A second secret would be weaker in practice, because users would pick something short or reuse it anyway.
- **The kernel can already verify it.** The encrypted vault (`apps/kernel/src/secret/vault.rs`) derives its key with Argon2id and authenticates its contents with AES-256-GCM. Verifying the passkey is the same derivation followed by an authentication check.
- **It is never given to agents.** The M26 vault plan (`docs/M26_CHARIOX_ENCRYPTED_VAULT_PLAN.md`) already guarantees that the passphrase is captured by a kernel-owned prompt and never delivered to an agent.
- **Chariox-wide.** One secret for every critical action on a kernel, whatever the surface: TUI, web, or CLI. It belongs to the kernel's vault, so a user with several kernels has one passkey per kernel unless they choose the same passphrase for each.

### 4.2 Verification without unlocking

Verification derives the key from the vault header's salt and KDF parameters, then authenticates the vault file with it. It must not touch the unlocked-vault map or change any unlock expiry. Unlock policies (`operation`, `ttl`, `kernel_init`, `always`, see `apps/kernel/src/runtime/state/runtime_vault_unlock_state.rs`) are independent of verification. A verified passkey neither unlocks a locked vault nor extends an unlocked one.

Argon2id is deliberately slow and memory-hungry. Verification runs on a blocking worker, and the kernel allows one verification at a time so that a flood of attempts cannot exhaust memory. Callers that queue behind it count toward the rate limit.

### 4.3 Rate limits

The Phase 1 limiter applies to every passkey check, including the new ones in this plan (token minting, `/sudo` entry): 5 failures, then an exponential lockout capped at 15 minutes. It is per owner and held in kernel memory, so a kernel restart clears it.

A same-user process can deliberately send wrong passkeys to lock the owner out for up to 15 minutes. That is a denial of service, not a bypass, and the cap bounds it. Once human surfaces carry their own credential (step 2 in section 9), attempts can be counted per credential class, so failures from an agent credential do not lock out the TUI.

### 4.4 Rotation

Changing the vault passphrase changes the passkey. When it changes, the kernel:

1. revokes every kernel access token,
2. ends every sudo window and every Phase 1 remember window,
3. ends every active sudoagent mode, and
4. appends an audit event for each revocation, plus one for the rotation.

### 4.5 Non-encrypted vault backends

If the vault backend is not the encrypted Chariox vault, there is nothing to verify the passkey against. Critical approvals already fail closed in Phase 1. This milestone does the same: no token minting and no `/sudo`. The user sees a clear message pointing to the encrypted vault setup. Whether to offer a standalone passkey for other backends is open question 11.

### 4.6 Remote and hosted kernels

The passkey belongs to the vault of the kernel that verifies it. For a remote or hosted kernel, the user types it in the web or remote TUI. It travels end-to-end encrypted through the relay (`apps/kernel/src/transport/relay_crypto.rs`) like any other kernel request, so the relay and the Cloud see only ciphertext and never store it. Kernel-to-kernel relay peers never carry a passkey on a user's behalf.

## 5. Kernel access tokens

### 5.1 When a token is minted

A token is minted only on a human surface, after the user enters the passkey, or during an active sudo window that was itself opened with the passkey. Typical triggers:

- the user enters `/sudo` on an agent (section 7),
- the user grants an agent or a session kernel access from the TUI or web ("give this session user-level access for 30 minutes").

An agent cannot mint a token, for itself or for anyone else. The kernel refuses a mint request from any connection that is not a human surface.

### 5.2 Format

- 32 random bytes from the OS random source, base64url encoded, with a fixed prefix (for example `chx_kat_`) so that log redaction and secret scanners can recognize it.
- Opaque. It carries no claims; everything about it lives in the kernel.
- The kernel stores only a SHA-256 hash of the token and compares in constant time. A fast hash is enough because the token has 256 bits of entropy.
- Each token also has a short, non-secret token id used in audit events, listings and revoke commands.

### 5.3 Scopes

| Field | Meaning |
|---|---|
| `level` | `user` (acts as the owner within scope) or `metaagent` (the sudoagent tool set: spawn, prompt and supervise agents, workflows, MCP installs and grants, credential handles, routine approvals). Whether two levels are needed is open question 13. |
| `session_id` | The one session the token is valid for. Required. There are no all-sessions agent tokens. |
| `agent_ids` | For a per-agent token, the agent it was minted for, plus any agents that agent spawns while the token is live. Empty for a per-session token, which is valid for every agent in the session. |
| `minted_by` | The human surface and user id that entered the passkey, plus the id of the passkey audit event. |
| `expires_at` | Absolute expiry (section 5.4). |

Whatever the level, a token never grants:

- answering critical approvals,
- minting or extending tokens,
- vault export or reading secret values (credential handles only, as today),
- changing the passkey or the kernel access configuration.

### 5.4 Lifetime

The proposed defaults follow the vault's TTL defaults and are for the owner to confirm (open question 4):

```toml
[kernel_access]
token_default_minutes = 30
token_max_minutes = 240
sudo_window_default_minutes = 15
sudo_window_max_minutes = 60
```

A token can be extended from a human surface before it expires. Extension needs the passkey unless a sudo window is open. It can never go past `token_max_minutes` from the original mint without a fresh passkey.

### 5.5 Revocation

A token is revoked, and its hash dropped, on any of:

| Trigger | Notes |
|---|---|
| Session end | Session close, archive or delete. |
| Agent removal | For per-agent tokens; also when the agent's provider run ends for good. |
| `/sudo` exit | Including exit on timeout (section 7.4). |
| Explicit revoke | From the TUI, the web, or the CLI by token id. "Revoke all" is always available. |
| Kernel restart | Tokens live in kernel memory only (section 5.6). |
| Passkey change | Section 4.4. |
| Expiry | `expires_at` reached. |

Revocation and expiry act immediately, not at the caller's next request. Under every option in section 6 the kernel records which connections and subscriptions each token authorized; on revocation or expiry it cancels those subscriptions (the subscription loop in `runtime_transport/subscriptions.rs` runs independently of inbound requests, so an idle subscriber would otherwise keep receiving transcripts and snapshots) and closes the connections. As a second line, each delivery and replay snapshot rechecks that its token is still live. A focused test revokes and expires a token while its client stays idle and checks that no later event or replay snapshot is delivered.

### 5.6 Storage and delivery

**In the kernel.** Only the hash and scope record, in memory. Nothing is written to disk. A kernel restart revokes everything, and the user re-enters the passkey. Persisting hashes across restarts is open question 7; the default proposal is not to.

**To the agent.** The token must reach the agent's process without being readable by other agents. Candidates:

| Delivery | Readable by other same-user processes? | Fits provider CLIs? |
|---|---|---|
| Environment variable in the provider launch env, inherited by the agent's shell and any `chariox` CLI it runs | Often yes: another same-user process can usually read it through process inspection (for example `/proc/<pid>/environ` on Linux) | Yes. It needs an explicit exception to the env scrubbing in `provider_launch_policy.rs` for this one variable. |
| A 0600 file in a per-agent directory | Yes, unless agents get read isolation (option D) | Yes |
| An inherited file descriptor | Harder to reach, but provider CLIs do not reliably pass descriptors to the tool shells they spawn | Poorly |
| No new secret: elevate the agent's existing runtime MCP identity | The per-run MCP bearer is already delivered through the provider's MCP configuration (see `generate_runtime_mcp_auth_token` in `apps/kernel/src/app/provider_launch_request.rs`); no second secret appears | Yes, for kernel access through MCP tools; not for a `chariox` CLI run from the agent's shell |

No delivery method keeps a secret from a determined same-user process on a laptop without read isolation. That is why the session-scoping check in section 6 must not rely on secrecy alone. The tentative proposal is to elevate the runtime MCP identity for MCP-based access, and to use the environment variable only when shell-level CLI access is wanted. Open question 2 asks the owner to choose.

### 5.7 Audit

Durable events, next to `critical_approval.passkey`, never containing the token or the passkey:

- `kernel_access.token` with outcome `minted`, `extended`, `revoked`, `expired` or `rejected`, plus the token id, level, session id, agent ids, the minting surface and the revocation reason.
- `kernel_access.sudo` with outcome `entered`, `exited`, `timed_out` or `refused`.
- `kernel_access.denied` for each request refused for scope. These are sampled and aggregated so a looping agent cannot flood the log.

The TUI and web show the live tokens for a session with their expiry and a revoke control.

## 6. Session scoping: an open question

The owner decided that tokens are per agent or per session. How the kernel makes sure a token is only used within that scope is not decided. There are three options, and they can be combined.

### 6.1 Option (a): connection-bound token

The token is redeemed once, on one websocket connection. From then on the authority belongs to that connection, and the token string is useless elsewhere. On loopback there is no TLS, so this is connection identity rather than cryptographic channel binding. Over the relay it could bind to the relay session keys.

- **Pros:** a token copied after redemption cannot be replayed. Revocation is exact: close the connection. Simple to implement.
- **Cons:** it does not scope anything by itself. A bound connection can still address any session unless (b) is also enforced. There is a race before first redemption: whoever redeems first wins. It fits long-lived connections like the TUI, but the CLI opens one connection per command, so each command would need a new token or a refresh step.

### 6.2 Option (b): kernel-side binding to the session's agents

The kernel keeps `token -> {level, session_id, agent_ids}`. For every request it computes the request's session scope, as `request_session_scope` in `apps/kernel/src/runtime/session_membership/scope.rs` already does (`SessionId`, `SessionIds`, `SessionRef`, `AttachmentId`, `AllSessions`), and refuses anything outside the token's session and agents. `AllSessions` requests such as `ListSessions` are filtered to the token's session. Requests with no session scope (global queries) are refused for scoped tokens unless explicitly allowlisted.

- **Pros:** it is the only option that actually enforces "this session only". It reuses existing scope machinery. It works the same over the local socket, the relay and the MCP listener, and is independent of the OS.
- **Cons:** every request type needs a correct scope mapping, and a missing or wrong mapping is a hole. Today `request_session_scope` returns `None` for global requests, so deny-by-default for scoped tokens is essential. A token stolen by another same-user process still works within its session until it expires or is revoked. New request types add maintenance cost; a test should fail when a request type has no decided scope.

### 6.3 Option (c): process identity check

The kernel identifies the connecting process and checks that it belongs to the provider process tree it launched for the token's agent. The OS reports the peer through `getpeereid` and `LOCAL_PEERPID` or `LOCAL_PEERTOKEN` (the macOS audit token, which avoids PID reuse), and `SO_PEERCRED` on Linux. These work only on Unix domain sockets, so this option means adding a Unix socket endpoint (mode 0600) next to, or instead of, the loopback TCP port. Mapping a TCP loopback connection back to a PID through the OS socket table is possible, but racy and platform-specific.

- **Pros:** a stolen token is useless from any other process. There is no secret to deliver, which also sidesteps section 5.6. The OS user check also stops other OS users outright.
- **Cons:** PID ancestry is fragile. Daemonized helpers reparent to `launchd` or `init`, provider CLIs spawn helpers and subagents, and sandbox wrappers or containers change the picture (managed workers run in containers with a different PID namespace). PID reuse needs audit tokens or start-time checks. A same-user process can still inject into the agent's process on many systems. It does not apply to remote clients at all, and Windows needs a different mechanism (named pipes and `GetNamedPipeClientProcessId`). It is the most platform code of the three.

### 6.4 Comparison

| | (a) Connection-bound | (b) Kernel-side binding | (c) Process identity |
|---|---|---|---|
| Enforces the session scope | No, needs (b) | Yes | No, needs (b) for request-level scope |
| Stops replay of a stolen token | After redemption only | No | Yes |
| Works over the relay and for remote clients | Partly (relay session) | Yes | No |
| Needs a secret delivered to the agent | Yes | Yes | No |
| Platform-specific code | None | None | Significant (macOS, Linux, Windows) |
| Fits the per-command CLI | Poorly | Yes | Yes, with a Unix socket |
| Main failure mode | Race before redemption | A request type with a wrong scope mapping | Reparented or containerized processes rejected; injection into the agent |
| Implementation cost | Low | Medium | High |

### 6.5 Tentative recommendation (needs the owner's decision)

Use **(b) as the base in every topology**, since it is the only option that enforces scope, and make it deny by default for requests with no mapped scope. Add **(c) as hardening for agent tokens on local kernels** once a Unix socket endpoint exists, starting with macOS and Linux. Process-group or shared OS-session membership alone is not an identity: on Linux any process in the same session can `setpgid` into another group. (c) therefore needs a dedicated OS session created at each provider launch (`setsid`) plus kernel-tracked descendant identity with PID-reuse protection (for example the pid with its start time, or a pidfd on Linux), and a negative test where a sibling agent's process joins the target's group and presents the stolen token. Use **(a) only for long-lived human-surface connections**, such as the TUI, where it costs nothing. This is a proposal, not a decision (open question 1).

## 7. The sudoagent

Today `/meta <task>` puts the focused agent into a temporary Meta mode that is delegation-only (`docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, catalog entry in `apps/kernel/src/runtime/terminal_command_catalog/catalog/core.json`). The metaagent can plan and supervise but cannot edit files or run shell tools. The owner's decision is to replace this with a sudoagent.

### 7.1 Entering

- `/sudo <task>` replaces `/meta <task>` on the focused agent.
- Entering requires the passkey, or an active sudo window. The sudo window works like the vault's TTL: when the user enters the passkey they may choose a window (default 15 minutes, max 60, per section 5.4), and further `/sudo` entries within it need no passkey.
- Entry mints a per-agent token at `metaagent` level (or `user` level if the owner chooses, open question 13), scoped to the agent's session.
- `/sudo` can only be typed on a human surface. An agent cannot put itself or another agent into sudo mode.

### 7.2 What the sudoagent can do

- Everything a normal agent does: edit files, run shell and script tools, use its MCP tools. It is no longer delegation-only.
- Kernel access within its token's scope: spawn, prompt and supervise agents in its session; create and run workflows; install and grant MCPs and skills; manage credential handles through kernel-owned flows; answer routine approvals (tool permissions, App binding approvals in Ask mode) as the metaagent can today.

### 7.3 What it still cannot do

- Answer critical approvals. Those always need a fresh passkey, or the Phase 1 remember window, on a human surface. The sudoagent can raise a request that leads to one; a human answers it.
- Mint, extend or hand out tokens, for itself or for other agents.
- Export the vault or read secret values.
- Change the passkey, the vault, or the kernel access configuration.
- Act outside its session.

### 7.4 Exit, timeout and audit

- The sudoagent leaves sudo mode when its task completes, when the user exits it (for example `/sudo off` or a control in the TUI and web), when its token expires, or on any revocation trigger in section 5.5.
- On exit the agent returns to normal mode and its token is revoked.
- Every transition appends a `kernel_access.sudo` event. Every kernel request made with the token is attributed to the token id in existing traces.

### 7.5 Compatibility for existing `/meta` users

- For one release, `/meta` keeps working and shows a notice that `/sudo` replaces it. `/meta` stays delegation-only and needs no passkey, because it grants nothing new.
- Meta tasks already running (`apps/kernel/src/runtime/state/metaagent_task_runtime_state.rs`) finish in Meta mode. They are never upgraded to sudo automatically, since that would grant authority without a passkey.
- In a later release `/meta` is removed from the command catalog and the CLI (`apps/cli/src/command-center.test.ts` covers it today). Whether a delegation-only mode survives under another name is open question 8.

## 8. Effect on each client

| Client | Credential to the kernel | Passkey prompt | Change |
|---|---|---|---|
| Web via relay | Cloud session, then relay end-to-end encryption; the kernel sees a `RelayClient` with a user id | Hidden-input dialog in the web; end-to-end encrypted to the kernel | Adds the `/sudo` entry dialog, the token list with revoke, and the sudo window countdown. The browser never holds a kernel access token. |
| TUI (local) | The local-kernel auth token (step 1), later a human-surface credential (step 2) | Hidden-input panel, as in Phase 1 | Reads the token file at start; shows sudo state and live tokens. |
| CLI, interactive | Same token file as the TUI | Prompted on a TTY with echo off | Commands that need the passkey prompt for it. |
| CLI, non-interactive (scripts) | Token file, or an agent token from the environment when run inside an agent's shell | None | Commands that need the passkey fail with a clear error when there is no TTY. There is no passkey flag or environment variable (open question 9 asks about stdin). |
| Remote TUI | Relay, like the web | Hidden input, end-to-end encrypted | Same as the web. |
| Kernel-to-kernel relay peers (`apps/kernel/src/transport/relay_peer.rs`) | Peer identity and relay keys | Never | Tokens are never forwarded to peers. A remote agent gets kernel access only from its own kernel. Relayed answers to kernel-operation decisions stay refused, as today. |
| Hosted and managed workers | `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`, as today, held by the host controller | Through the web, end-to-end encrypted | Unchanged. That token keeps its meaning as the host controller's credential. Agent tokens are layered on top, and managed isolation already keeps the host token from agents. |

The same local-token pattern is used for package actions in `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md` (random kernel auth, authenticated handshake, token removed before provider launch). This plan brings it to the laptop kernel.

## 9. Migration steps

Each step ships on its own and leaves the system working.

1. **Local-kernel auth token (option A), defense in depth.** On start, the laptop kernel writes a random token to a 0600 file in its state directory and accepts it on the handshake `Authorization` header, as managed workers already do. The TUI and CLI read the file. At first, connections without it are accepted but logged. This stops other OS users once enforced, and prepares every client.
2. **Human-surface credentials.** The kernel records which credential class each connection used (host token, human surface, agent), counts passkey failures per class, and attributes audit events to it. `is_terminal_caller` becomes a statement about credential class rather than transport source.
3. **Agent tokens.** Minting, scopes, lifetime, revocation, delivery and audit (section 5), with the session-scoping enforcement the owner chooses (section 6). Tokens are optional: nothing requires them yet.
4. **Require credentials on the local socket.** Unauthenticated loopback connections are refused. This needs a future protocol version so that old clients get a clear upgrade message instead of a bare 401. Managed workers are unaffected.
5. **`/sudo`.** The sudoagent (section 7) ships alongside `/meta`, with the notice on `/meta`.
6. **Retire `/meta`.** Remove it from the catalog, the CLI and the tool set after one release. This also needs a future protocol version, because the command catalog and interaction shapes change.
7. **Optional hardening.** The Unix socket endpoint and process identity check (option (c)) for local agent tokens, and read isolation (option D) if the owner pursues it.

## 10. Threat model

### 10.1 Assets

- Pending decisions, above all critical approvals.
- The passkey (the vault passphrase) and the vault contents.
- Kernel control: spawning and prompting agents, workflows, App installs and bindings.
- Session data: transcripts, events, artifacts.
- Kernel access tokens and the audit log.

### 10.2 Actors

| Actor | Description |
|---|---|
| Same-user agent process | An agent's shell or tool, or any process it starts, running as the owner's OS user on a laptop kernel. The main concern of this plan. |
| Other OS users | Accounts on the same machine that can reach loopback. |
| Browser pages | Pages in the owner's browser that try to open a websocket to loopback. |
| Compromised relay | A relay or Cloud component that an attacker controls. |
| Remote peer | Another kernel connected through the relay. |
| Malicious App | An installed App, its backend or its workflows, trying to escalate through the kernel. |

### 10.3 Attacks and mitigations

| Attack | Mitigation |
|---|---|
| A same-user agent connects to the socket and answers a critical approval | Phase 1: the passkey is required at approval time; agents never see it. |
| A same-user agent connects to the socket and acts as the owner in other ways | Steps 1 to 4: credentials required; agent tokens scoped by (b); optionally bound to the process by (c). |
| Another OS user connects to loopback | Step 1 token in a 0600 file; later a 0600 Unix socket with a peer UID check. |
| A browser page drives the kernel over loopback | #618 refuses any upgrade with an `Origin` header. |
| A compromised relay reads or replays the passkey or a token | End-to-end encryption; the relay sees ciphertext only. Tokens are never sent to or through peers. Passkey fields are zeroized after use. |
| A remote peer answers the owner's decisions | Already refused for kernel-operation decisions (#443 tests); peers never receive tokens. |
| A malicious App escalates through its workflows | App workflows never receive tokens. App validation is a critical approval and needs the passkey. |
| An agent steals another agent's token | (b) confines it to that token's session; (c) makes it useless from another process; short lifetimes and revocation bound the window. |
| An agent tries to mint a token or enter `/sudo` | Minting and `/sudo` are accepted only from human-surface credentials, with the passkey. |
| Online guessing of the passkey | 5 failures, then exponential lockout capped at 15 minutes; one Argon2id verification at a time. |
| An agent locks the owner out with wrong guesses | A DoS bounded at 15 minutes; per-class counting after step 2. |
| Prompt spoofing: an agent prints a fake passkey prompt in its transcript | Passkey prompts appear only in kernel-owned UI with a distinct look, never inline in agent output; the docs tell users never to type the passkey into an agent's input. |
| A token leaks into logs or transcripts | Fixed prefix for redaction and scanning; the kernel stores only the hash; tokens never appear in audit events. |

### 10.4 Residual risks

- **Memory and input capture.** A same-user process that can read the kernel's memory, attach a debugger, or log keystrokes can capture the passkey or tokens. On macOS keystroke capture needs a TCC permission; under X11 on Linux any client can read keystrokes. This plan does not address it.
- **File reads by agents.** Without read isolation (option D), local agents can read the token file from step 1 and the vault file. The token file then only stops other OS users, and the vault file enables offline guessing of the passkey, slowed only by Argon2id. Passphrase strength matters more now that it gates approvals (open question 10).
- **Token theft within scope.** With (b) alone, a token stolen by another same-user process works within its session until it expires or is revoked.
- **Remember and sudo windows.** Within an open window a human approves or enters sudo without retyping the passkey. The windows are short, off or minimal by default, and still require a human surface.
- **Process identity gaps.** With (c), processes that reparent or live in another PID namespace are rejected (safe but inconvenient), and code injection into the agent's process defeats the check.

## 11. Open questions for the owner

1. Session-scoping enforcement: adopt the tentative recommendation of (b) everywhere, (c) for local agent tokens, and (a) for long-lived human connections? Or a different combination?
2. Token delivery: elevate the agent's existing runtime MCP identity (no new secret), inject an environment variable for shell-level CLI access, or both?
3. Default granularity: should `/sudo` and the grant UI default to per-agent or per-session tokens?
4. Lifetimes: confirm the token default of 30 minutes and max of 240, and the sudo window default of 15 minutes and max of 60.
5. Should the sudo window and the Phase 1 remember window stay separate, or become one "presence window"?
6. On laptops, accept that the human-surface token file is readable by same-user agents until read isolation exists, or add an OS presence check (option C, Touch ID or polkit) for `/sudo` entry and token minting?
7. Keep tokens in kernel memory only, so that a restart revokes them all, or persist hashes so tokens survive restarts?
8. How long should `/meta` stay as an alias, and should a no-passkey delegation-only mode survive under another name?
9. May the CLI read the passkey from stdin for automation, or only from an interactive TTY?
10. Should the kernel enforce a minimum passphrase strength now that the passphrase is the passkey, and prompt existing users with weak passphrases to rotate?
11. For non-encrypted vault backends: fail closed permanently, or offer a standalone passkey?
12. Is Windows in scope for the process identity check, or is (c) macOS and Linux only?
13. Are two token levels (`user` and `metaagent`) needed, or should the sudoagent simply get `user` level within its session?
14. Should the grant UI allow tokens for sessions shared with other users, and if so, whose passkey mints them?

## References

- Trust-boundary analysis and options A to D: `/Users/miguel/.codex/evidence/chariox-apps-phase1/approval-trust-boundary/README.md`
- Phase 1 passkey evidence: `/Users/miguel/.codex/evidence/chariox-apps-phase1/passkey-critical-approvals/`
- Kernel socket and local token: `apps/kernel/src/runtime_transport.rs`
- Terminal caller check: `apps/kernel/src/runtime/command/caller.rs`
- Session membership and request scoping: `apps/kernel/src/runtime/session_membership.rs`, `apps/kernel/src/runtime/session_membership/scope.rs`
- Encrypted vault and unlock policies: `apps/kernel/src/secret/vault.rs`, `apps/kernel/src/runtime/state/runtime_vault_unlock_state.rs`, `docs/M26_CHARIOX_ENCRYPTED_VAULT_PLAN.md`
- Metaagents: `docs/M23_METAAGENTS_PLAN.md`, `docs/M26_METAAGENTS_DELEGATION_ONLY_PLAN.md`, `apps/kernel/src/runtime/state/tool_dispatch/meta.rs`
- Runtime MCP listener and per-run bearer: `apps/kernel/src/transport/mcp_server.rs`, `apps/kernel/src/app/provider_launch_request.rs`
- Package action local token: `docs/DEPLOYED_WORKFLOWS_THREAT_MODEL.md`
