# External Agents v2 Plan

**Status:** Approved by Miguel, 2026-10-20 (all eight questions answered, none open). Plan only, no code. Protocol numbers below are placeholders (`NEXT`, `NEXT+1`); the coordinator allocates them.

**Decisions taken on the open questions (Miguel, 2026-10-20):**

1. One local passkey grant reaches all of the user's machines; no per-machine narrowing. The popup says so and revoke is immediate.
2. The remote kernel also enforces the human-only set when a request carries `on_behalf_of` (section 5.3). One shared function; no Cloud and no link involved.
3. Machine file requests (`Read/Write/StatMachineFile`) are added, chunked, resumable, sha256-checked, 2 GiB cap (section 9.1).
4. b3 becomes a real signed kernel in Miguel's hosted account through a one-time ssh bootstrap. This is an interim step: it is replaced when b3 becomes a Path-1 Chariox-managed machine or a BYOM machine.
5. Verify first, then decide: Cloud already lists kernels for **user** sessions (waiting room, remote TUI picker). Step 1 checks whether a **kernel** credential can call that list. A kernel-scoped read-only variant is added only if it cannot (section 3.3).
6. Approval deep link (128-bit, single use, 10 min, account and kernel bound) ships as an extension after the core. Self-hosted uses a remote TUI until the native mobile client exists.
7. `apps-external-agents` is closed out as in section 10.
8. The popup default stays 8 h; "until the agent exits" is opt-in; a kernel restart still ends it.

Builds on `docs/CHARIOX_KERNEL_ACCESS_PLAN.md` (D1–D16), `docs/KERNEL_SUDO.md` and `docs/KERNEL_ACCESS.md`. Where this plan changes a rule from those documents it says so (section 4).

## 1. Summary

Today an external agent (Claude Code, Codex, OpenCode started by the user, not by Chariox) can get one passkey-approved grant and then act as the user on **its own local kernel only**. v2 makes four changes:

1. A grant can last **until the agent process exits**.
2. The local kernel becomes a **gateway**: the agent names a target machine and the local kernel forwards the request over the relay, exactly as a remote TUI would. The agent never talks to Cloud or the relay.
3. Machines are **discovered by the local kernel**, like a terminal does it. The agent sees `chariox machines` and `--machine <name>`.
4. **Passkey approvals never pass through the agent.** The agent only receives a pointer; the user answers in a Chariox client. (Extension, after the core.)

The goal behind it: the coordinator (Claude Code on Miguel's Mac) works on the Mac and on b3 through this surface and drops ssh to b3, except as break-glass for a kernel that is down.

## 2. Goals and non-goals

### Goals

- One passkey grant on the local kernel gives an external agent the user's ordinary authority on every machine the user's own terminals can reach.
- Everything stays kernel-owned: the local kernel is the only thing the agent talks to; the remote kernel is the authority for its own sessions, agents, terminals and files (AGENTS.md rules 1, 3, 4).
- Every outcome is definitive: success, refusal with a reason, or "machine offline/unreachable" with what to do next. No silent fallback to another machine or to local.
- Revocable and visible from any Chariox client, on the machine that granted it.
- Nothing installed into the agent's harness. The agent learns the surface from `chariox guide`.

### Non-goals

- Cloud sign-in, tokens or Cloud calls by the agent. An external agent requires a local kernel (no kernel, no access; ssh is break-glass).
- Isolating agents from the user's own files or ssh keys (D12 stands). A same-user shell agent could already run `ssh`; v2 does not add power over that, it makes the access explicit, audited and revocable with one passkey.
- Agents answering passkey approvals, minting grants, reading secret values (D16, D3 stand).
- Kernel-to-kernel peer requests from agents. The gateway speaks the **client** protocol a terminal speaks, never the kernel peer protocol.
- Selkies/noVNC work, a root agent in Cloud (stratum 4), a native mobile client.
- Windows peer identity.

## 3. The five decisions and how each maps onto code

### 3.1 Grant lifetime "until the agent process exits" (decision 1)

**What exists.**
- `RequestKernelAccessRequest { holder_pid, lifetime_minutes }` (`apps/kernel/src/local/api/types/kernel_access.rs`). The grant record `Grant` (`runtime/kernel_access.rs`) has a `deadline: Instant`; `sweep_kernel_access` ends a grant when the deadline passes or `holder.alive()` is false.
- Holder = nearest ancestor that is a provider harness (`kernel_access/requester.rs::default_holder`), identified by PID + path + start time + exec version (`kernel_access/process.rs::ProcessIdentity`). Descendants are admitted; a new process with a reused PID is not (`access_ended_for`).
- Popup lifetime comes from the approve answer's numeric `custom_reply` (up to `grant_max_minutes` = 1440); an "ends in 5 minutes, extend?" popup follows.

**What changes.**
- A grant has a **term**: `Minutes(n)` or `UntilHolderExits`. One enum replaces `lifetime_minutes` on the request, on `KernelAccessGrant`, and in `Grant`; `deadline` and `expires_at_ms` exist only for `Minutes`. Asking for both is an error.
- The popup offers three choices: "until the agent exits", "8 hours" (default), "24 hours". The 8 h default and 24 h cap stay.
- `UntilHolderExits` grants have no extension popup. They end on holder exit, explicit revoke, passkey rotation or kernel restart (grants stay in kernel memory, so a kernel restart still means a new grant; this is the cost of "nothing on disk").
- **Exit detection (built in step 1, no new watcher).** Liveness is the existing identity check (PID + path + start time) in `sweep_kernel_access`. It already runs on every list, on every request admission, on the 100 ms tick of each open holder connection and on the kernel pump, so the grant and its subscriptions end as soon as anyone looks, without a pidfd/kqueue watcher. Measured on b3 with a real Claude Code harness killed by exact PID: grant list empty after 30 ms, orphaned former descendant refused after 32 ms (EA2-01/02, evidence `extagent/r3/green-impl-1`). macOS uses the same check; its drill is a Mac cell.
- Binding is unchanged: ProcessIdentity compare, descendants admitted, no inheritance by a new process.
- Grants stay listed (`ListKernelAccessGrants`) and revocable (`RevokeKernelAccessGrant`) from every Chariox client. The list shows the term.

**Deleted.** `lifetime_minutes` on the request and on the grant summary; `lifetime_minutes` stays on the popup shape (`PasskeyPrompt`) only as the minutes copy of a minutes term, for clients that predate the typed `term` (the hosted web popup in `chariox-cloud` reads it until it is updated), and for sudo windows. The `custom_reply` answer is one parser, `KernelAccessTerm::from_reply` (`until_exit` or minutes), checked once at the answer boundary.

### 3.2 Everything, but only through the local kernel (decision 2)

**What exists.**
- The Unix socket admits a request only for a bound grant holder (`runtime_transport/unix_access.rs`); every request passes `authorize_external_request` (`runtime/state/kernel_access/policy.rs`). Its refusal list contains the pairing, relay, Cloud identity and `ResolveKernelClientConnection` requests, and the docs say a grant cannot "attach to a REMOTE kernel or issue kernel-peer requests" (`docs/PROTOCOL.md`, Kernel access).
- A frame has no machine. A holder can already ask the local kernel for leased remote workers (`MoveAgentToRemote`, `ListRemoteMachines`), which is the local kernel's own business.
- A remote TUI gets a relay URL and a short-lived relay token from its kernel (`ResolveKernelClientConnection`, `IssueCloudRelayClientToken`; hosted) or a kernel-issued terminal grant (`TerminalPairingLinkJoined`, `terminal-grants.json`; self-hosted), then speaks the normal encrypted client protocol to the target kernel as a `RemoteClient`.

**What changes.** A new gateway inside the local kernel (section 5). The agent keeps one connection: the Unix socket. A frame may carry `machine`; the local kernel authorises the request locally (same policy function), then forwards it as a terminal-class client of the target kernel and returns the response. No relay token, key or Cloud credential ever reaches the agent; `ResolveKernelClientConnection`, `IssueCloudRelayClientToken` and the pairing/relay requests stay refused.

**Deleted.** The "no remote kernel" rule in the access plan (D6 stratum 2 "only the local kernel", D9) and in `PROTOCOL.md`; the per-component "remote not supported" notes on the external path (section 4 lists them). What stays refused is the credential-bearing set, not "remote".

### 3.3 Discovery like terminals (decision 3)

- **Hosted.** The local kernel fetches its owner's machine list from Cloud (heartbeat freshness), the same list the web waiting room and the remote TUI picker show. Relay target selection stays Cloud's job (AGENTS.md rule 4).
  - Cloud already lists kernels, but the open point was whether a **kernel** credential, the durable one that already renews terminal tokens, may call that list.
  - **Verified in step 1 (2026-10-20, `chariox-cloud` main `00658b85a`, OSS `landing/oss-r1`): yes, nothing is added to Cloud.** The list is `GET /relay/targets?accountId=&realmId=` (`apps/api/src/routes/relay.ts` → `relay/service.ts::listRelayTargets` → `relay/targets-repository.ts::listRelayTargetsForRealm`). Its auth takes a user session bearer **or** `x-chariox-kernel-credential` **or** a machine credential (`http/relay-directory-credential.ts`). With a kernel credential it requires that the credential's realm equals the requested realm and that its user is a member of the account. It returns every `DAEMON` target of the realm with `daemonId`, alias, `machineId`, machine alias, `status`, `lastHeartbeatAt` (freshness applied), providers and `acceptingLeases`. The kernel already calls it with the kernel credential: `get_cloud_kernel_directory` (`apps/kernel/src/runtime/cloud_api_client/http.rs`), used by relay discovery and the remote inventory behind `ListRemoteMachines`. So the hosted inventory source reuses the existing remote-inventory projection; the one limit is that it is realm-scoped (machines of the kernel's own realm), which is what "the user's machines" means today.
- **One source interface.** The gateway's machine list sits behind a single inventory-source interface with one job: "give me the user's machines with name, status, last seen, protocol and how to reach them". Two sources implement it today, the hosted Cloud list and the paired kernels below. The gateway, the CLI and the agent guide only ever see the interface, so a future self-hosted registry or a kernel gossip mesh (Miguel's idea, a separate future area) can be added as a third source without changing the gateway or the CLI. No such source is built in v2.
- **Self-hosted.** The machines are the kernels this kernel paired with through terminal pairing links. A person runs `/machine add <link>` (new, human-only) in a Chariox terminal on the local kernel; the kernel performs the terminal join (`JoinTerminalPairingLink` logic) with its gateway key and stores the machine in `gateway-machines.json` (0600). No Cloud involved.
- **Agent-facing:** `chariox machines`, `--machine <name>` on CLI commands, `chariox guide`. Names are the user's local aliases (`/machine rename`); ambiguity is an error that lists the candidates, never a guess. Only machines the user's terminals could reach count; unapproved relay peers are not targets.

### 3.4 Approvals (decision 4)

- Routine approvals may be answered by the agent (unchanged, on any machine).
- Passkey approvals (grant, `/sudo`, critical) are never answered by the agent. A passkey field in a gateway request is refused locally and by the remote kernel (the existing `Refusal::HumanOnly`).
- The kernel raises the interaction; the agent receives a **pointer without secrets**; the user answers in an already-paired Chariox client (D1 popup). Details and extension scope are in section 8.

### 3.5 Dropping ssh (decision 5)

Section 9 lists what the coordinator needs and which area provides each piece.

## 4. What is deleted or rewritten

| Item | Action |
|---|---|
| `CHARIOX_KERNEL_ACCESS_PLAN.md` D6 stratum 2 "only the local kernel", D9, §5.4 "may not attach to another kernel", §6.2 | Rewrite: stratum 2 reaches the user's machines through the local kernel. Strata 3 (sudo) stay per kernel. |
| `PROTOCOL.md` Kernel access paragraph ("attach to a REMOTE kernel or issue kernel-peer requests") | Rewrite: remote reach through the gateway; peer requests still refused. |
| `KERNEL_SUDO.md` "External agents ... over that kernel's Unix socket" | Add `--machine`; sudo requests for remote agents raise the popup on the remote kernel. |
| `lifetime_minutes` (request, grant summary, internal `Grant`) | Replaced by the typed term. |
| Popup text "access to this kernel" | "this kernel and your other machines (N online)". |
| Tests asserting a remote refusal for grant holders | Replaced by the EA2 tests below. |
| `docs/KERNEL_ACCESS.md` | Short section for agents and the guide command. |

Not deleted: the denylist for credential-bearing and identity requests, response redaction for credential registry and MCP literals (`runtime/external_response.rs`), the one-pending-request rule, the "ended" holder memory (EXT-UX-05).

## 5. Gateway design

### 5.1 Parts (all in the local kernel, new module `runtime/machine_gateway/`)

- **Directory.** Resolves a machine name to `{kernel_id, machine_id, relay_url, target daemon id, protocol versions, freshness}` through the inventory-source interface (3.3): the hosted Cloud list or the paired kernels. Refreshed in the background (existing remote-inventory projection), never on the request path.
- **Gateway identity.** One key pair per local kernel, generated by the kernel on first use, stored privately in kernel state, never returned by any request. Hosted: it is registered as a terminal client named "Agent gateway of <machine>" so it appears in the paired-client list of every machine it reaches and can be revoked there. Self-hosted: it is the key the pairing link pinned.
- **Connections.** One relay client connection per target machine, opened lazily on first use, shared by all grants, closed after 5 minutes idle. End-to-end encryption uses the same `relay_crypto` as a remote TUI; the relay sees ciphertext only. Hosted settings that break the rules fail loudly: a non-`wss://` non-loopback relay is refused with a clear message (AGENTS.md rule 6).
- **Forwarding.** The Unix socket handler (`unix_access.rs`) sees `machine` on a frame, runs `authorize_external_request` locally, then hands the request to the gateway. Responses and subscription events come back on the same agent connection, tagged with the machine. Subscriptions use the existing resumable event ids; the gateway cancels the remote subscription when the grant ends.
- **Recheck.** Every delivery rechecks that the grant is live (existing `delivery_live`), so revoke/expiry/holder exit stops forwarding at once.

### 5.2 Credentials the local kernel uses

| Mode | For the relay | For the remote kernel |
|---|---|---|
| Hosted | A short-lived, key-bound relay client token minted from the kernel's durable Cloud credential (same path as terminal pivot tokens; silent renewal per #927). Stays in kernel memory. | The remote admits the gateway key as an owner terminal (relay `user_id` pin, as for any owner terminal). |
| Self-hosted | Operator transport token (shared or scoped CLIENT token) given by a human at `/machine add`, stored 0600 in `gateway-machines.json`, never printed, never returned by a request. | The terminal grant the pairing link created (`terminal-grants.json`); revocable on the remote. |

The agent holds nothing, so there is nothing to copy (D9 intact).

### 5.3 Permission handling on the remote kernel

The remote sees an owner terminal and handles requests exactly as for a remote TUI (ownership checks, session membership, D1 popups). Two additions:

- The forwarded request carries `on_behalf_of: { origin_kernel, grant_id, harness }` inside the encrypted request context. The remote kernel makes its own decision from it; no Cloud and no link are involved. The link between the machines is only used afterwards, when the user wants the action done and answers in a client.
- **Three checks, not one.** Today `authorize_external_request` does all of this in one call against the local grant map and the local `pending_interactions`. That cannot run on both ends: B has no record of A's grant, and A does not own B's interactions (for an interaction id A does not know, `external_interaction_requires_terminal` returns "not forbidden", so A cannot be the one to classify a remote answer). It is split into three functions:
  1. `grant_live(grant_id)`: **origin only.** A checks its own grant (live, holder covers the peer). B never sees or looks up a grant id; `on_behalf_of.grant_id` is audit text on B.
  2. `restricted_request(request)`: **shape only, shared.** The credential/pairing/relay/Cloud-identity/peer set, protected config paths, and any `RespondToInteraction` that carries a `passkey`. It reads no state, so both ends call the same function. A runs it before forwarding; B runs it again for every request that carries `on_behalf_of`.
  3. `interaction_requires_terminal(answer)`: **state, target only.** Whoever owns the interaction classifies it from its own `pending_interactions` (passkey prompt, terminal credential, critical level, choice requiring a passkey). For a request with `machine`, A skips this step and B performs it with `on_behalf_of` present; routine approvals are admitted, anything else is refused with `kernel_access_human_only`. For a local request nothing changes.
- **Compatibility.** A target kernel below protocol `NEXT` ignores `on_behalf_of` and cannot run step 3. A therefore refuses `RespondToInteraction` to such a target with `machine_incompatible` (naming the minimum protocol) and forwards all other requests (they are fully decided by steps 1 and 2 at A). A newer target never trusts the absence of `on_behalf_of` from a gateway caller: a request from a connection registered as an "Agent gateway" without `on_behalf_of` is refused.
- A popup for something that needs the **remote** kernel's passkey appears on the remote's connected clients (D1, per-kernel passkey). The agent gets the pointer (section 8) naming the machine.

### 5.4 Failure and offline behaviour (definitive outcomes)

Each failure is a typed error with a code, one plain sentence and the next step. Codes (new, same style as `Refusal`):

| Code | When | Next step in the message |
|---|---|---|
| `machine_unknown` / `machine_ambiguous` | Name matches none / several | Run `chariox machines`. |
| `machine_offline` | No fresh heartbeat (hosted) or relay shows it gone | Start the kernel there; ssh is break-glass. |
| `machine_unreachable` | Relay or connect timeout (10 s) | Retry; check the relay. |
| `machine_unpaired` | Self-hosted: not paired, or pairing revoked there | Re-pair with a new link. |
| `machine_incompatible` | Remote protocol below the minimum for this request | Update that kernel; names the minimum. |
| `machine_request_timeout` | No answer within the request deadline (default 30 s, per request override) | For state-changing requests: retry once with the same `--command-id` (below). |
| `machine_outcome_unknown` | Retry found a reservation without a receipt (target crashed mid-request) | Check the state before asking again; the request is not re-run. |
| `machine_request_not_retryable` | State-changing request kind that is not on the target's ledger | Do not retry blindly; check the state. |
| `machine_connection_lost` | Connection dropped with a request or subscription in flight | Same. |

- **Retries: at most once, with an explicit "unknown".** The existing `CommandResultCache` is not an exactly-once ledger: ordinary reservations live in memory only, results are written after completion, entries are bounded (512 entries, 24 h on disk), and several request kinds are excluded. After a crash on B between the side effect and the stored result, a retry with the same command id could run the mutation again. The plan therefore promises less and says so:
  - **Read-only requests** (list, stat, read, `read --since <cursor>`) are retried freely. **Chunk writes** (`WriteMachineFile` with offset and chunk sha256) are idempotent by construction: the same chunk at the same offset yields the same bytes.
  - **State-changing requests** go through a durable gateway ledger on the target, built on the at-most-once receipt path that app control already uses (`CommandResultCache::new_at_most_once`, `complete_at_most_once`): the reservation `(gateway caller, command_id, request fingerprint)` is appended and synced **before** the request is dispatched, and the final receipt after it. Retention 24 h, bounded capacity; a full ledger refuses new mutations with a typed error instead of dropping entries.
  - Outcomes of a retry with the same `command_id`: receipt present → the original result; reservation present but no receipt (B crashed or restarted mid-request) → `machine_outcome_unknown`: "the request may or may not have run; check the state (for example `sessions list`) before asking again". The request is **never re-executed** from an interrupted reservation. A different fingerprint under the same id is a conflict error. A request kind that cannot be put on the ledger is refused through the gateway with `machine_request_not_retryable`, rather than offering a retry that could run twice.
  - The agent guide says: on `machine_request_timeout` or `machine_connection_lost`, retry once with the same `--command-id`; on `machine_outcome_unknown`, look at the state first.
- **Never a hidden fallback.** A failed named machine never routes to local or to another machine.
- Revoking the grant stops new requests and subscriptions at once. Work already started on the remote (a running terminal command, a started agent turn) continues, as it does for a local grant; it is stopped with an explicit request.
- Local kernel down: no access at all (ssh is break-glass, recorded as a product issue).

### 5.5 Audit

Local: `kernel_access.forward` per request (request kind, machine id, outcome, latency; no payload), sampled like `kernel_access.denied` when looping. Remote: its ordinary command audit, with the gateway caller and `on_behalf_of`. No passkey or credential value in either.

## 6. Protocol changes

Per AGENTS.md: version bump, snapshot and hash tests, a focused drill. Numbers are allocated by the coordinator.

**Local protocol `NEXT` (core).**
- Access term: `RequestKernelAccessRequest.term` and `KernelAccessGrant.term` replace `lifetime_minutes`; `expires_at_ms` is optional.
- Unix-socket frames: `machine` (optional) on request and subscribe frames; `machine` echoed on `Response` and event frames; new error codes of 5.4 in `KernelTransportError`.
- Self-hosted machine list: `JoinMachineLink { link, relay_token? }` (human-only), `ForgetGatewayMachine`, and `ListGatewayMachines`. Hosted needs no new request if step 1 confirms existing listing.
- Machine view for agents: extend `RemoteMachinesListed` rows with `status`, `last_seen`, `kernel_protocol_version`, `via` (hosted/self-hosted) if they are not already there.
- Relay client request context: `on_behalf_of` (5.3).
- Files (section 9): `ReadMachineFile`, `WriteMachineFile`, `StatMachineFile` with offset, length, sha256, mode.
- Client minimums: raised only for clients that show the grant list (term) and the popup choices; the web and TUI popups need the three-choice answer.

**Relay peer protocol.** Verified in step 1: the relay client envelope (`KernelIncomingFrame::Request { request_id, command_id, causation_id, correlation_id, request }`, `transport/kernel_protocol.rs`) is not peer-versioned and has no `deny_unknown_fields`: a kernel that predates a new optional field (`machine`, `on_behalf_of`) ignores it silently, which is why 5.3 defines the compatibility rule on the origin side. The *relay peer* protocol versions kernel-to-kernel `RelayPeer` messages, which the gateway never sends. It is still bumped together with the local protocol in this repo, because the peer union embeds local response shapes (the term change touches `KernelAccessGrant`, which appears in `KernelSudoTurn.requester`); the relay binary itself does not change.

**Local protocol `NEXT+1` (approvals extension).** `ApprovalPending` frame to the waiting agent connection, `ListPendingApprovals` (read-only for agents), pointer record `{approval_id, kind, machine, one_line, link?, expires_at}`; Cloud API `POST /v1/approval-pointers` (chariox-cloud).

Drills for these shapes: section 12 (EA2-01..EA2-19). Snapshot and hash tests next to `protocol_shapes/kernel_access.rs`.

## 7. CLI surface

All commands print one JSON document on stdout, errors as JSON on stderr (`{"error":{"code","message","machine"}}`). Exit codes: 0 ok, 2 usage, 3 refused (human-only, not covered, ended), 4 machine unavailable, 5 timeout/outcome unknown, 6 approval pending. Final numbers fixed at implementation.

```
chariox guide                                  # short agent guide (below)
chariox access request [--until-exit | --minutes N] [--no-wait]
chariox access status | list | revoke <id|--all>
chariox machines                               # name, status, last_seen, kernel protocol, via
chariox request [--machine M] [--command-id ID] < request.json   # one LocalDaemonRequest
chariox sessions|agent|terminal|files ... [--machine M]          # thin verbs (below)
chariox sudo request --agent A --prompt P [--machine M]
chariox approvals list | wait <id> [--timeout S]                 # extension
```

- `chariox request` is the single generic door: any `LocalDaemonRequest` the grant allows, locally or on `--machine`. It is the same shape as the sudo MCP `chariox_kernel_request`, so there is one request vocabulary.
- Thin verbs only where the raw request is awkward for an agent: `terminal open|send|read|wait|close`, `files get|put|stat`, `agent spawn|prompt|steer|read`, `sessions list|create`. Each is a one-screen mapping onto a request; none adds behaviour. `--machine` is accepted by every verb and by the existing non-TUI commands that talk to a kernel (`context`, `app`). `chariox logs` reads local files today; remote logs are read with `terminal`/`files get` in the core.
- No "current machine" state: every call names its machine, or means local.
- `chariox guide` is static text compiled from the same command table the parser uses, so a test fails if the guide mentions a command or flag that does not exist. It tells the agent: request access once, list machines, always pass `--machine`, which things are human-only, and **what to do with an approval pointer: show it to the user verbatim and wait**.
- The default holder for `access request` stays the nearest provider harness. `--no-wait` exists because harness tool calls time out (minutes): it prints the pointer and exits 6, then `access status` polls.

Nothing is written into `~/.claude`, `~/.codex`, MCP config or hooks (checked by EA2-19).

## 8. Approvals extension

The core already has: popups on the kernel that owns the passkey (D1), `RequestKernelAccess`/`RequestKernelSudo` blocking with a one-line note on stderr ("Waiting for you to approve in your Chariox terminal on <kernel>").

The extension adds, when the user is not looking at a terminal of that kernel:

1. **Pointer.** When an external request raises a passkey interaction (local or remote), the kernel sends the waiting connection `ApprovalPending { approval_id, kind, machine, one_line, link?, expires_at }`. The CLI prints it as the first line of output so the agent shows it in its chat. It contains no secret and no prompt body.
2. **Hosted answer.** If the kernel is Cloud-enrolled, it asks Cloud (kernel credential) for a one-time deep link: 128-bit code, single use, 10 minutes, bound to `(owner account, kernel id, interaction id)`. Cloud stores only that mapping and the kind label. Opening it serves the Chariox web app's approval view and authenticates the account as today. The web client then connects to the kernel through the relay (end-to-end encrypted) and shows the same popup the D1 popup shows; the passkey goes browser → relay → kernel. Cloud serves the page; the relay never serves pages or sees the passkey. A second open, a wrong account, an expired or answered link all show a plain "already answered / expired" page.
3. **Self-hosted answer.** No page. The pointer says which kernel to open; the user answers in a remote TUI paired to it (works from a phone SSH app). The native mobile client takes this role when it exists.
4. **Phishing note.** The agent prints the pointer, so a hostile agent could print a lookalike link. The real popup is also already shown on every connected client, and the page shows details from the kernel, not from the link. The guide tells agents to quote the pointer verbatim; the user is told in the pointer text to prefer opening Chariox itself if the domain is not the one they use.

The extension can ship after the core; the core's popups cover the Mac TUI, and `--no-wait` covers harness timeouts. Remote passkey prompts (a remote `/sudo` request, a remote critical approval) work in the core when someone has a client connected to that remote kernel; the extension makes them reachable from a phone.

## 9. What the coordinator needs to drop ssh

| Need | Provided by | Gap to close |
|---|---|---|
| Run commands, incl. long ones that outlive the agent's call | Room terminals (`chariox-terminal` #1054, term/r1: open/list/send/read/wait/close, Room-owned, survive client disconnect, background jobs count as work, protocol 562 / peer 170 pending) | #1054 must be on main; add the `terminal` verbs (7); `read --since <cursor>` and `wait --timeout` for detached jobs. **Verified in step 1** (term/r1 `ac9db6927`, `docs/PROTOCOL.md` Room terminals): a terminal is opened with `RoomTerminal { session_id, acting_agent?, operation: open }`. It needs no agent (without `acting_agent` the actor is the user; `host_agent: null` means the Room's home kernel), but it is **Room-scoped**: it needs a session (Room) on that machine, runs in the Room's project folder, and only the Room's owner may send it (the gateway is an owner terminal). So the coordinator keeps an ops Room per machine (`sessions create --machine b3`), and the `terminal` verbs take `--session`. No machine-level terminal exists; none is added. |
| Start/steer sessions and agents on a machine | Existing `CreateSession`, `SpawnAgent`, prompt/steer requests via the gateway | `agent`/`sessions` verbs; machine tag on events. |
| Read files, logs, run `git` | Terminal (`cat`, `tail`, `journalctl`) works with no new code | Convenience `files stat/get`. |
| **Transfer files, incl. git bundles and artifacts** | Nothing general today (`DIRECTORY_WORKSPACE_TRANSFER` and managed context transfer only move provisioned contexts; `ReadFileCapabilityRequest` is session-scoped text) | New small family `Read/Write/StatMachineFile`, see 9.1. A git bundle is then `files put` + `git fetch bundle` in a terminal. |
| Logs of the kernel itself | Terminal / `files get` | A dedicated logs request is not in v2. |
| Build slots, drills, killing exact PIDs | Terminal | None. |
| Slices | Home kernel's slice requests (`ListSlices`, Room/Environment requests) and their terminals via the home kernel | Verify a slice on a remote machine is driven through that machine's kernel with `--machine`. |
| Machine is a real kernel in Miguel's account | b3 runs a signed product kernel as a per-user service, enrolled to his hosted account (owner-managed machine / Setup) | One-time ssh bootstrap, then ssh is break-glass. **Interim (Miguel, 2026-10-20):** this hand enrolment is replaced when b3 becomes a Path-1 Chariox-managed machine or a BYOM machine; nothing in the gateway depends on how the machine was enrolled. |

### 9.1 Machine file requests: why and how

**Why.** The coordinator's day on b3 moves files in both directions: git bundles from b3 to the Mac so the Mac can publish them (b3 has no GitHub credentials by design), lane results and `LANE_STATUS.md`, evidence (screenshots, recordings, logs), and briefs or prompts going the other way. Pushing these through a terminal as base64 is slow, breaks on large files, cannot resume, and fills the agent's context with bytes. A terminal is also the wrong tool: it is a live shell, not a transfer.

**How.**
- They are ordinary kernel API requests (`ReadMachineFile`, `WriteMachineFile`, `StatMachineFile`), not terminal traffic. The gateway routes them to the target kernel like any other request with `machine`; the target kernel does the file work and answers.
- Chunked: each request moves one bounded chunk (offset, length). A large file is a sequence of chunks, streamed with relay backpressure.
- Resumable: `Stat` returns size and sha256 of what is already there; the client continues at the offset after an interruption (drill EA2-07 interrupts at 40 %).
- Verified: the final sha256 is compared by the receiving kernel; a mismatch is a typed error and the partial file is never presented as complete.
- Capped: 2 GiB per file; a larger file is refused with a typed error naming the cap.
- Authority: the user's, no wider. A terminal on that machine can already read and write any file the OS user can; the requests add convenience, not power. The bytes never reach the model unless the agent chooses to read them, and everything stays inside the relay's end-to-end encryption.
- CLI: `chariox files get|put|stat --machine <name>`, JSON result with size, sha256 and elapsed time.

Every break-glass ssh during the dogfooding day (EA2-16) is logged as a product issue with the missing capability.

## 10. Closing out `apps-external-agents`

Source: acceptance tracker `ACCEPTANCE.json` rows `apps-external-agents/*` and `AREAS.json` (PRs chariox#910 and chariox-cloud#308; owner lane b3:kaext; waiting on val910 r3 for the hosted soak and a Miguel session). #910 landed with OSS main #934, Cloud #308 with Cloud #315, and #927 (remote TUI token renewal) with #934, so the "waiting on #927" blocker is gone.

| Row | Status today | Decision |
|---|---|---|
| KA-01 holder identity, descendant/sibling/PID reuse | PASS (matrix partly not rerun) | **Keep** as the local regression; the full matrix reruns in EA2-02 (Linux) and on the Mac. |
| KA-02 whole-kernel grant, human-only/remote/secret negatives | FAIL (blocked: real signed App and approved critical operation) | **Replace**: local part → EA2-05; "remote negative" turns into a positive; human-only negatives → EA2-10. The signed-App critical-operation negative moves into EA2-10 (needs the same blocked resource; stays BLOCKED(coordinator) until a signed App exists). |
| KA-03 one sudo popup, yield ends authority | PASS | **Keep**; EA2-11 adds the remote sudo request. |
| KA-04 rotation/revoke/expiry/holder exit | PASS; open findings: raw "Connection ended" for an in-flight call, uncleared revoke toast, web reconnect churn after Caddy 4 h close | **Replace** with EA2-01/02/03/12; the two UX findings become fix items in step 1 (definitive in-flight outcome, toast cleared on revoke). |
| KA-HOSTED-20M 20 min zero socket closures | "NOT RUN on current candidate" | **Retire**; folded into the EA2-14 soak (2 h, same zero-closure rule, now with the gateway). |
| EXT-UX-01..05 popup, notices, countdown, refusal text | PASS (various heads) | **Keep** as regression for the local path; EXT-UX-05's reason-code list extends with the machine codes (EA2-09, EA2-10). EXT-UX-02 (extension notice) does not apply to until-exit grants. |

Process: the `val910` lane and its "val910 r3 + Miguel attended session" next step are retired; Miguel's attended session becomes EA2-16. A new area `external-agents-v2` with the rows below replaces `apps-external-agents` once KA-01/03 and EXT-UX-* are re-run green on the new head. Landing PRs: chariox#910 (landed), chariox-cloud#308 (landed). The tracker update is the coordinator's.

## 11. Build order

1. **Core A, local only.** Access term + until-exit detection, popup choices, `--until-exit`, `--no-wait`, in-flight outcome and revoke-toast fixes, doc rewrites. (EA2-01..03)
2. **Core B, gateway hosted.** First check whether a kernel credential can read the Cloud machine list (3.3). Inventory-source interface, directory, gateway identity, connections, frame `machine`, error codes, `on_behalf_of`, shared policy, `machines`, `request`, `guide`, `--machine`. (EA2-04, 05, 09, 10, 11, 12)
3. **Core C, self-hosted.** `/machine add <link>`, `gateway-machines.json`, scoped-token path. (EA2-08)
4. **Core D, work surface.** Terminal verbs on top of #1054; file family; slice check. (EA2-06, 07, 13, 15)
5. **Dogfood.** EA2-14, 16, 19 on the real Mac + b3.
6. **Extension, approvals.** Pointer frame, `approvals`, Cloud pointer endpoint, web approval view, self-hosted remote-TUI path. (EA2-17, 18)

Steps 1–3 can be separate PRs; step 6 includes a `chariox-cloud` PR. One build per machine at a time (AGENTS.md).

## 12. Validation scenarios

All are real-path, red-capable drills (they fail on the base or before the step, pass after) driven the way a user does it, with evidence per step outside the repo. Rigs:

- **Rig H (hosted):** per-PR stack on b3 (`/w/valenv/pr-stack`, hosted Cloud + `wss` relay behind Caddy), kernel **A** ("Mac": user-domain) and kernel **B** ("b3": the remote), both enrolled to the same fixture account through the product login, real TUI on A (and a web client at DPR1/DPR2), real harness as the agent (Codex `gpt-6-luna` low, Claude `claude-haiku-5-5`), uplink shaped to 8 Mbit/s and ≥ 60 ms RTT on the WAN cells.
- **Rig S (self-hosted):** operator relay (`apps/relay`) with no Cloud process at all, A and B paired by a terminal pairing link.
- **Mac cells** (identity uses `LOCAL_PEERTOKEN`) are run by the coordinator on Miguel's Mac; everything else on b3 or in a b3 KVM guest.
- Each cell records the measured RTT, the resolved model id, kernel/CLI versions and commit.

| ID | Scenario | Red before | Passes when |
|---|---|---|---|
| **EA2-01** | **Until-exit grant.** Real Codex/Claude on A requests access; user picks "until the agent exits" in the TUI popup and on web. | No such choice. | Grant is listed with the exit term and no expiry, has no deadline (still valid after the 8 h default has passed, checked with a test clock and on a real run), and the exact harness PID being killed ends the grant and closes its idle subscriptions within 2 s; `list` and the web list agree. |
| **EA2-02** | **Binding and PID reuse.** After EA2-01's exit, a new harness is started that gets the same PID (Linux `ns_last_pid` in a PID namespace); a sibling process; a child shell of the harness. | n/a (negative) | New process and sibling refused (`kernel_access_ended`/`not_covered`); child admitted. Mac: same on macOS. |
| **EA2-03** | **Revoke and list from any client.** Revoke the grant from TUI, web (hosted), and a remote TUI. | in-flight call returns raw "Connection ended" | Each client lists it and revokes it; an in-flight request ends with a definitive `kernel_access_ended`; no stale toast. |
| **EA2-04** | **Agent never contacts Cloud.** The agent process runs in a network namespace with no route out; only the kernel's Unix socket is visible. It runs `chariox machines`. | No `machines`. | Machines list shows B online (heartbeat age), an offline machine as offline, and the agent's packet capture shows zero outbound traffic; the agent environment holds no token. |
| **EA2-05** | **Remote session via gateway (hosted).** From A: `--machine b` create a session, spawn a real agent, prompt, read, steer, stop. A web client attached to B watches the same session live. | `--machine` unknown. | All succeed; B's audit shows the gateway caller with `on_behalf_of` the agent; events carry the machine tag; the local session list is unchanged. |
| **EA2-06** | **Remote terminal, detached job.** `terminal open --machine b`, start a 10-minute job, kill the agent and CLI, restart them, `read --since`, `wait`. Needs #1054. | n/a | Output is complete and ordered, exit code returned, job survived agent exit (unlike its grant: grant ended, a new grant is needed). |
| **EA2-07** | **File and git bundle transfer.** `files put` a 200 MB bundle to B over the shaped link, interrupt at 40 %, resume, verify sha256, `git fetch` it in a B terminal, `files get` a result back. | No file family. | Hash equal, resume continues at the offset, size cap refuses with a typed error, throughput recorded, relay frames stay encrypted. |
| **EA2-08** | **Self-hosted.** Rig S: `/machine add <link>` (human) then the EA2-05 flow. Revoke the pairing on B. | no gateway | Zero Cloud connections during the run; after revoke the next call returns `machine_unpaired`. Scoped-token variant too. |
| **EA2-09** | **Offline and failing machines.** (a) stop B's kernel (exact PID) mid-request; (b) unknown and ambiguous names; (c) relay blocked; (d) B on the base commit (old protocol); (e) **crash after the side effect, before the receipt**: a test hook on B kills the kernel (exact PID) right after `CreateSession` took effect and before its receipt is written. | Raw errors / hangs | Codes `machine_offline`, `machine_unknown`/`ambiguous`, `machine_unreachable`, `machine_incompatible`, `machine_request_timeout`, each with its next step, within 10 s (connect) / 30 s (request). For (e): after B restarts, a retry with the same `--command-id` returns `machine_outcome_unknown` and does **not** create a second session; for a read-only request and for a request that crashed *before* its side effect the retry succeeds; a request that completed returns its original result. Nothing ran on A. |
| **EA2-10** | **Human-only through the gateway, and the positive twin.** Agent tries on B: answer a critical approval, send a passkey, request a grant, read a secret/credential literal, `ConfigureRelay`, `ResolveKernelClientConnection`, a kernel-peer request, and raw frames without `--machine`. Includes a signed-App critical operation when available. **Positive:** B raises a routine Room question (no passkey, not critical); the agent answers it with `--machine b` and the answer takes effect. | n/a | Each negative refused with the exact code at A and, via a patched A that skips its check, also at B by `restricted_request` / `interaction_requires_terminal` (the `on_behalf_of` set); audit shows each; no state changed. The routine answer is admitted although A has no such interaction and B has no such grant. Against B on the base commit: the answer is refused at A with `machine_incompatible`; other requests still work. |
| **EA2-11** | **Remote `/sudo` request.** `sudo request --machine b`; popup on B's connected client; user types B's passkey; one turn runs; yield ends it. | n/a | Same single-turn rule as KA-03 on B; the agent never sees the passkey. |
| **EA2-12** | **Gateway ends with the grant.** An open remote subscription and an in-flight remote terminal read when the grant is revoked, expires, and the holder exits. | n/a | Both end at once; later events are not delivered; the remote terminal command keeps running (documented). |
| **EA2-13** | **Slice.** Docker slice (compatibility mode on) on A driven locally; a slice on B driven through `--machine b`. | n/a | Create, terminal, agent turn in each; Room terminals work through the home kernel. |
| **EA2-14** | **Soak.** Real Codex or Claude drives A and B for 2 h on the shaped hosted path; B's kernel restarted once, A's relay connection dropped once. | n/a | Zero stuck states, zero hosted web socket closures (any closure is a FAIL), every failure definitive, resource samples flat. Replaces KA-HOSTED-20M. |
| **EA2-15** | **Work surface parity.** Everything the coordinator does on b3 in a normal day, scripted (build in a slot, read lane status, move a bundle, start a drill, kill an exact PID) through the CLI only. | n/a | All steps pass without ssh. |
| **EA2-16** | **Coordinator dogfooding day.** The coordinator (Claude Code on Miguel's Mac) works on the Mac and b3 for a day with ssh to b3 withdrawn (key not available to the agent), grant "until the agent exits". | n/a | Every break-glass use is a recorded product issue; PASS when none is open at the end of the day and Miguel reviews the log. |
| **EA2-17** | **Approvals from a phone (extension, hosted).** An agent requests a grant on A, then a `/sudo` on B; the pointer appears in the agent chat; Miguel opens the link on a real phone (DPR 3); types the passkey; the kernel proceeds. Repeat with a canary passkey. | No pointer. | Works end to end; the link is single-use, expires at 10 min, refuses the wrong account and the wrong kernel; Cloud logs, relay capture and the agent transcript contain no passkey (canary grep); the real popup was also visible on the TUI. |
| **EA2-18** | **Approvals self-hosted (extension).** Pointer says which kernel; user answers in a remote TUI over an ssh phone app (second host). | n/a | Same outcome; no web page involved. |
| **EA2-19** | **Guide only.** A fresh Codex and a fresh Claude agent get one line: "run `chariox guide` and ..." and complete tasks on B (list sessions, run a command, move a file) with no human hints. | No guide. | Both succeed; a test checks the guide against the parser; no files appear in the harness directories (`~/.claude`, `~/.codex`, MCP config). |

Supplementary regression (not acceptance): Rust tests for the typed term, frame `machine`, policy-sharing, error codes and the snapshot/hash shapes; `pnpm protocol:conformance`; `pnpm lint:process-signals` for the exit watcher; CLI guide-vs-parser test.

## 13. Risks and residual notes

- **Blast radius.** One local passkey reaches all machines. This follows decision 2; the popup says so, the list shows it, the remote paired-client list shows the gateway, and revoke is immediate. Narrowing per machine is not in v2.
- **Kernel restart ends grants.** By design (memory only). For the coordinator this means re-granting after a Mac kernel restart.
- **Remote passkeys are per kernel.** Remote prompts need the user at some client of that kernel; the extension makes that practical.
- **Dependency:** #1054 for terminals; a Cloud change only if step 1 finds that a kernel credential cannot read the owner's machine list (3.3).
- **UX-first rule applied:** no new guard on what the agent reads or types; the only fences are the existing human-only set and the existing credential-literal response redaction.
