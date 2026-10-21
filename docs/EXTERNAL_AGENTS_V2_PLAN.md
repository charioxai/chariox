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
- **Connections.** One relay client connection per target machine, opened lazily on first use, shared by all grants, closed after 5 minutes idle and replaced after 4 minutes of age (hosted client tokens last 5 minutes). End-to-end encryption uses the same `relay_crypto` as a remote TUI; the relay sees ciphertext only. Hosted settings that break the rules fail loudly: a non-`wss://` non-loopback relay is refused with a clear message (AGENTS.md rule 6).
- **Forwarding.** The Unix socket handler (`unix_access.rs`) sees `machine` on a frame, runs `authorize_external_request` locally, then hands the request to the gateway. Responses and subscription events come back on the same agent connection, tagged with the machine. Subscriptions use the existing resumable event ids (they count on the target's stream, so a resume sends the last one seen with the machine); the gateway cancels the remote subscription when the grant ends or the agent unsubscribes, and renews the relay token in place so a subscription outlives the 5-minute token.
- **Attachments the gateway opened (decision of 10-20, built in r5).** An `AttachToSession` answered by a machine is kept alive by the origin kernel, not by the agent: while the grant is live and the machine answers, the origin sends `HeartbeatAttachments` for all such attachments every 10 s (one batched request, answered with the ids that no longer exist). They end when the grant ends, three beats are missed, or the agent sends `DetachFromSession`; the target sweeps a silent attachment after 30 s as always. The agent never re-attaches as a chore.
- **Recheck.** Every delivery rechecks that the grant is live (existing `delivery_live`), so revoke/expiry/holder exit stops forwarding at once.

### 5.2 Credentials the local kernel uses

| Mode | For the relay | For the remote kernel |
|---|---|---|
| Hosted | A 5-minute, key-bound relay client token minted from the kernel's durable Cloud credential through the existing terminal pivot path (`issue_cloud_terminal_client_token` with a kernel credential: `allow_unpaired_client_subject`, exact target, key thumbprint). Stays in kernel memory. The link is replaced before expiry instead of renewed. | The remote admits the gateway key as an owner terminal (relay `user_id` pin, as for any owner terminal). |
| Self-hosted | Operator transport token (shared or scoped CLIENT token) given by a human at `/machine add`, stored 0600 in `gateway-machines.json`, never printed, never returned by a request. | The terminal grant the pairing link created (`terminal-grants.json`); revocable on the remote. |

The agent holds nothing, so there is nothing to copy (D9 intact).

### 5.3 Permission handling on the remote kernel

The remote sees an owner terminal and handles requests exactly as for a remote TUI (ownership checks, session membership, D1 popups). Two additions:

- The forwarded request carries `on_behalf_of: { origin_kernel, grant_id, harness }` inside the encrypted request context. The remote kernel makes its own decision from it; no Cloud and no link are involved. The link between the machines is only used afterwards, when the user wants the action done and answers in a client.
- **Three checks, not one.** Today `authorize_external_request` does all of this in one call against the local grant map and the local `pending_interactions`. That cannot run on both ends: B has no record of A's grant, and A does not own B's interactions (for an interaction id A does not know, `external_interaction_requires_terminal` returns "not forbidden", so A cannot be the one to classify a remote answer). It is split into three functions:
  1. `grant_live(grant_id)`: **origin only.** A checks its own grant (live, holder covers the peer). B never sees or looks up a grant id; `on_behalf_of.grant_id` is audit text on B.
  2. `restricted_request(request)`: **shape only, shared.** The credential/pairing/relay/Cloud-identity/peer set, protected config paths, and any `RespondToInteraction` that carries a `passkey`. It reads no state, so both ends call the same function. A runs it before forwarding; B runs it again for every request that carries `on_behalf_of`.
  3. `interaction_requires_terminal(answer)`: **state, target only.** Whoever owns the interaction classifies it from its own `pending_interactions` (passkey prompt, terminal credential, critical level, choice requiring a passkey). For a request with `machine`, A skips this step and B performs it with `on_behalf_of` present; routine approvals are admitted, anything else is refused with `kernel_access_human_only`. For a local request nothing changes.
- **Compatibility (built in step 2, 2026-10-20).** A target kernel advertises `machine_gateway_v1` (in `RelayStatus.capabilities` and in relay presence) when it enforces `on_behalf_of`. The relay client envelope parser of an older kernel has `deny_unknown_fields`, so it *rejects* a request that carries `on_behalf_of` rather than ignoring it. The gateway therefore sends `on_behalf_of` only to targets that advertise the capability. To an older target it sends the plain envelope for everything except `RespondToInteraction`, which A refuses with `machine_incompatible` (the only request that needs the target's own classification). The planned "a newer target refuses a gateway caller that omits `on_behalf_of`" defence is **not built**: the hosted client subject is a hash, so the target cannot tell a gateway from another terminal of the same owner; a compromised gateway is a compromised kernel anyway.
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
| `machine_not_available` | The frame came from something that is not a grant holder | Use the local kernel's own connection. |
| `machine_connection_lost` | Connection dropped with a request in flight, or a subscription's events stopped (the message names the last event id) | Same retry rule for requests; for a subscription, subscribe again with `resume_from_event_id`. |

- **Retries: at most once, with an explicit "unknown".** The existing `CommandResultCache` is not an exactly-once ledger: ordinary reservations live in memory only, results are written after completion, entries are bounded (512 entries, 24 h on disk), and several request kinds are excluded. After a crash on B between the side effect and the stored result, a retry with the same command id could run the mutation again. The plan therefore promises less and says so:
  - **Read-only requests** (list, stat, read, `read --since <cursor>`) are retried freely. **Chunk writes** (`WriteMachineFile` with offset and chunk sha256) are idempotent by construction: the same chunk at the same offset yields the same bytes.
  - **State-changing requests** go through a durable gateway ledger on the target, built on the at-most-once receipt path that app control already uses (`CommandResultCache::new_at_most_once`, `complete_at_most_once`): the reservation `(gateway caller, command_id, request fingerprint)` is appended and synced **before** the request is dispatched, and the final receipt after it. Retention 24 h, bounded capacity; a full ledger refuses new mutations with a typed error instead of dropping entries.
  - Outcomes of a retry with the same `command_id`: receipt present → the original result; reservation present but no receipt (B crashed or restarted mid-request) → `machine_outcome_unknown`: "the request may or may not have run; check the state (for example `sessions list`) before asking again". The request is **never re-executed** from an interrupted reservation. A different fingerprint under the same id is a conflict error. Read-only is decided by one rule (a request variant that starts with List, Get, Search or Stat); every other request is on the ledger, so there is no "not retryable" kind. A request without a `command_id` is refused with `command_id_required` (the CLI always sends one).
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
- Unix-socket frames: `machine` (optional) on request and subscribe frames; `machine` echoed on `Response` frames (built, so a client can tell a forwarded answer from a local one); event frames carry it when they come from a forwarded subscription (built in r5; one subscription per connection); `HeartbeatAttachments` / `AttachmentsHeartbeated` keep gateway attachments alive; new error codes of 5.4 in `KernelTransportError`.
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
- Chunked: each request moves one bounded chunk (offset, length) of at most 256 KiB, so it fits the relay's 768 KiB request bound after base64 (found in the r5 drill: 1 MiB chunks were refused). A large file is a sequence of chunks.
- Resumable: `Stat` returns size and sha256 of what is already there; the client continues at the offset after an interruption (drill EA2-07 interrupts at 40 %).
- Verified: the final sha256 is compared by the receiving kernel; a mismatch is a typed error and the partial file is never presented as complete.
- Capped: 2 GiB per file; a larger file is refused with a typed error naming the cap.
- Authority: the user's, no wider. A terminal on that machine can already read and write any file the OS user can; the requests add convenience, not power. The bytes never reach the model unless the agent chooses to read them, and everything stays inside the relay's end-to-end encryption.
- CLI: `chariox files get|put|stat --machine <name>`, JSON result with size, sha256 and elapsed time.

Every break-glass ssh during the dogfooding day (EA2-16) is logged as a product issue with the missing capability.

### 9.2 Terminal verbs on top of #1054 (built in r6, provisional protocol 569 with the Room terminal service)

Terminals are Room-scoped, owner-sent and survive client disconnects (term/r1, protocol 562 / peer 170). The verbs are one-screen mappings onto the single request `RoomTerminal { session_id, acting_agent?, operation }`; the gateway forwards it like any request with `--machine`. The agent calls exactly these (no `acting_agent`, so the actor is the user and the gateway, an owner terminal on the target, may type):

| Verb | Request (`operation`) | Notes |
|---|---|---|
| `sessions create --machine b --name ops --folder /root/work` | `CreateSession` (existing) | The ops Room per machine; its id is the `--session` of every terminal verb. |
| `terminal open --machine b --session S [--cols 120 --rows 30] [--title T] [-- ARGV...]` | `open { cols, rows, title?, command? }` | `command` is an argv (no shell); without it the user's login shell. Answers the terminal summary (id, `host_agent`). |
| `terminal list --machine b --session S` | `list` | Room-wide: this kernel's terminals plus workers'. |
| `terminal send --machine b --session S --terminal T "make test\n"` | `snapshot`, then `input { data_base64 }` | `input` is written as one unit. The CLI first takes the terminal's current `output offset` from a `snapshot` and prints it as `after_offset` so the following `read`/`wait` sees only the new output. Whether `input` itself returns the offset is checked on the #1054 base; if it does, the `snapshot` goes away. |
| `terminal read --since OFFSET [--wait-ms N]` | `read_output { after_offset, wait_ms }` | `gap: true` means resync: the CLI then runs `snapshot` and says so. This long poll replaces a subscription for terminals. |
| `terminal wait --for finished\|output_contains=TEXT\|waiting_for_input --timeout S` | `wait` | One request waits at most 240 s; the CLI loops for longer waits and prints the exit code when the shell reports one. |
| `terminal close` | `close` | Ends the shell's whole session (background jobs too). |

- **As built (r6).** `terminal open|list|send|read|wait|close` in `apps/cli/src/terminal-verbs.ts`, JSON on stdout, `--machine` optional (without it the local kernel answers). Differences from the table: `send` appends Enter (a carriage return; `--no-enter` types the text as it is), reads one `snapshot` first and prints its `output_offset` as `after_offset` (the `input` answer carries no offset); `read` prints the output as plain text (colour, title and cursor escapes removed) with `next_offset` and `gap`; `wait --timeout S` (default 60) asks in pieces of at most 240 s, each continuing from the offset the last piece saw, and a timeout is an answer (`fired: null` with the screen), not an error. The origin's wait for the machine's answer is the usual 30 s plus the request's own wait (`deadline_for` in `machine_gateway/ledger.rs`).
- **At-most-once (built).** The target's ledger treated a request as repeatable by the name of its variant (`List`, `Get`, `Search`, `Stat`, `Heartbeat`). `RoomTerminal` is classified by `operation`: `list`, `snapshot`, `read_output`, `wait` repeat; `open`, `input`, `resize`, `close` are state-changing and use the ledger, so a retried `input` never types twice.
- **Detached jobs.** The job belongs to the kernel-owned terminal, not to the agent or its grant: killing the agent ends the grant, not the job. A new grant (the same `--machine`, the same `--session`) reads it with `read --since`. A terminal is not an attachment, so the gateway's attachment heartbeat is not involved.
- **No new fence.** Anything typed or read is the agent's own action on a machine the user connected to it; the response redaction applies as to every reply.

### 9.3 Remote `/sudo` (design only; EA2-11)

A sudo window is a *kernel-side* decision on the machine that owns the agent and the work, so the remote case is not a new kind of sudo; it is the same request arriving through the gateway.

- **Whose interaction:** the target's (B). `RequestKernelSudo { agent_id, prompt }` is forwarded with `--machine b`; B raises its ordinary sudo popup (kind `sudo`, kernel-authored `SudoRequestPresentation` naming the requester as "<harness> via <origin kernel>") to its own connected host terminals, with B's passkey. The origin kernel only validates its grant and the request shape (it already refuses a sudo window itself as a gateway holder: "a sudo window does not reach other machines" stays).
- **Where the passkey is answered:** in any client already connected to B (TUI, web through the relay, a remote TUI over ssh). It never touches A, the gateway link, the agent or the model: `RespondToInteraction` with a passkey is human-only on both ends, and B classifies the interaction itself.
- **What B changes:** (1) the dispatch check "only a grant holder over ws+unix can request sudo" also admits a request that carries a valid `on_behalf_of` (authorized with `authorize_on_behalf_of_request`, not a local grant lookup); (2) a sudo turn started that way is bound to `(origin_kernel, grant_id)` instead of a local process identity: the gateway link's requests that carry the same `on_behalf_of` are the ones that run inside the window; (3) the presentation names the origin.
- **Ending:** one turn as for KA-03: yield, expiry or revoke on B ends it. When the origin grant ends, the gateway sends the yield to B as a best effort; B's own window deadline is the backstop (stated in the popup).
- **Agent-side result:** the forwarded request answers like a local one (pending, then approved or refused/expired); the agent is told "approve on <machine>" and nothing else. The deep-link pointer remains the approvals extension.
- **To verify at build:** that the kernel-authored presentation needs no local process facts, and that a worker/lease check in the sudo path does not assume the requester is local.

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
3. **Core C, self-hosted.** `/machine add <link>`, `gateway-machines.json`, scoped-token path. (EA2-08) **Built in r5**: human-only `JoinMachineLink` / `ForgetGatewayMachine`, TUI `/machine add|linked|unlink`; the target key learned at first contact is pinned; a scoped relay lists the paired machines with the CLIENT token given at pairing.
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

- **The gateway on the target (r6).** The target records each gateway that sends a request carrying `on_behalf_of`, keyed by the relay's authenticated client key (`gateway-origins.json`, 0600), and lists it beside its paired clients as `Agent gateway of <origin kernel>` (client id `agent-gateway:<thumbprint>`). Revoking that id (`RevokePairedClient`) refuses the key at once, with or without `on_behalf_of` (`machine_unpaired` at the origin). Self-hosted: a second pairing link for the same gateway key replaces the earlier grant (one grant per key; the replaced link cannot bring it back). **Not built:** the hosted gateway is not a Cloud client, so it is not in the Cloud clients list and the web's Cloud-side revoke does not reach it; a Cloud client needs a pairing session, which a kernel credential does not have. Hosted revoke today = revoke the grant on the origin, revoke the gateway on the target, or unlink the origin kernel. Decision for the coordinator: add a kernel-authenticated Cloud route that registers the gateway key as a client named "Agent gateway of <machine>" (small, Cloud only) or accept the target-side list.
- **Measured in r6 (debug builds, host load 55-100): not yet a product number.** On a shaped link (8 Mbit/s, 61 ms RTT, KVM guest) a 200 MB bundle moved at 1.7-2.0 Mbit/s up and 1.7 down (resume, sha256 and `git fetch` on the target all correct), forwarded requests on one persistent connection had P95 91-122 ms against a gate of RTT + 60 = 121 ms, and terminal echo (send then read) p50 174-255 ms. Plain TCP on the same link reached 7.1-7.5 Mbit/s, and unshaped the file path reached only 4.5 Mbit/s, so the limit is CPU on a loaded host with unoptimised crypto/base64, not the protocol (one 256 KiB chunk is 350 KB on the wire, 0.35 s at 8 Mbit/s, plus one RTT per chunk). The numbers need a release build on a quiet host before they count; pipelining chunks is the lever if they still fall short.
- **No request waits for the machine list (r6).** The directory used to refresh the Cloud machine list inline when it was more than one heartbeat old, so any forwarded request after a pause of about 10 s stalled for 1-6 s. It now answers from a list that is at most 5 minutes old and renews it behind the request; only an unknown or offline name, or no list at all, waits for a fresh one. Persistent-connection max latency after a 12 s pause went from 5850 ms to 43-201 ms.
- **Web (r6).** An access request opens its popup by itself on the web (once per request; closing leaves the footer notice) and Settings > Security shows the live grants of the selected kernel with Revoke / Revoke all (`ListKernelAccessGrants` / `RevokeKernelAccessGrant`).

- **Blast radius.** One local passkey reaches all machines. This follows decision 2; the popup says so, the list shows it, the remote paired-client list shows the gateway, and revoke is immediate. Narrowing per machine is not in v2.
- **Kernel restart ends grants.** By design (memory only). For the coordinator this means re-granting after a Mac kernel restart.
- **Remote passkeys are per kernel.** Remote prompts need the user at some client of that kernel; the extension makes that practical.
- **Dependency:** #1054 for terminals; a Cloud change only if step 1 finds that a kernel credential cannot read the owner's machine list (3.3).
- **UX-first rule applied:** no new guard on what the agent reads or types; the only fences are the existing human-only set and the existing credential-literal response redaction.
