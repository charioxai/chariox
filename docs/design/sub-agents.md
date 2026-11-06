# Sub-agents and the new shape of agent reach

Status: design for the product owner, 2026-11-06 (MP-08 / MP-09 / MP-10 / MP-11). No product code. Base: Landing 4 union head `cd799d2e60`, local protocol 999, relay peer 438. Code references are `apps/kernel/src/...:line` at that head. "OPEN" marks a question for the owner (section 10).
Words: **top parent** = the user-created agent at the root of a tree; **sub-agent** = any descendant; **peer** = two agents that are not ancestor and descendant; **decision** = a tool-permission request or question card.

## 1. What we build, in eleven lines

1. One edge, `parent_agent_id`, makes a tree. Only the user can create or change it. An agent that spawns in its own room creates a sub-agent.
2. Sub-agents are not in the room list, not counted in `max_agents`, and have no panel. The top parent's panel can show them on demand.
3. A decision raised by a sub-agent goes to its parent, then up the chain, then to the user. Nine decision kinds never leave the user (3.4).
4. A child never has more authority than its parent, checked at spawn and at every control command and decision answer, under the same lock as the action (section 4).
5. A parent controls its whole subtree without /sudo: deadline, cancel, steer, stop, usage, profile, files and history (section 5).
6. A subtree ends with its parent unless the user orphans it.
7. Three relations only: user, clients, external agents and assistants act with *user authority* within their grant; a parent has *delegated authority* over its subtree; peers can only *message* and *register to events* (section 6).
8. Today's reach (an owner-level `request` passthrough guarded by /sudo) is retired. Acting as the user belongs to external agents and assistants.
9. A remote sub-agent is the existing leased agent: the tree stays on the home kernel, so remote and local behave the same.
10. Everything is event-driven. A parent parks on `events.yield`; the kernel resumes it when a child finishes or needs a decision. Nothing polls.
11. Provider-native sub-agents (Claude Task, Codex spawn_agent) stay inside their provider and are not lineage edges.

## 2. Data model (a)

**Today.** `AgentInstance.spawned_by_agent_id` is an immutable creator receipt (`agent/types.rs:212`, `assert!` at `:339`). `direct_child` (`runtime/room_tool_admission.rs:29`) gives destructive rights over one level, same room. An older edge, `controlled_by_metaagent_id` (`agent/types.rs:209`), serves Meta mode (`tool_dispatch/meta/session_tools.rs:24-57`). Clients already receive the lineage field and ignore it (`packages/kernel-client/src/kernel-types-runtime.ts:440`). Spawned agents bypass the room limit (`agent/service.rs:198`) yet are counted when a user adds one (`:192-196`).

**Decision 2.1 — one edge.**
- `parent_agent_id: Option<String>` replaces `spawned_by_agent_id` and `controlled_by_metaagent_id`. Depth and root are derived by walking up, never stored. Depth cap 4 (OPEN Q1).
- Only a new user-only request `SetAgentParent { agent, parent | none }` changes it ("promote", "orphan"). Every agent caller is refused, like the pairing requests (`runtime/state/kernel_access/policy.rs:283-321`). No agent can adopt or reparent. An orphan stays in the top parent's room as an ordinary agent.
- Spawn sets the edge once, only from the spawner's current provider run (`runtime/state/room_agent_admission.rs:36-70`).
- No alias for the old field (development-stage policy): old stored agents lose their lineage and show as ordinary agents.

**Decision 2.2 — hosting.** A sub-agent needs a `session_id` for its worktree, history and interactions, so it keeps the top parent's room id and is never exposed as a member (an implementation fact). The kernel filters it out of `ListAgents`, the archived list and the snapshot's `agents()` (`runtime/session_read_control.rs:237-246`). A new read `ListDescendants { agent }` serves the top parent's panel. Room counts (`agent/service.rs:97,192`) skip agents with a parent.

**Persistence and restart.** The field is stored with the agent and in the `agent.created` event (`runtime/state/agent_lifecycle_owned_state.rs:34-45`), so it survives restart. A subtree ends with its parent: destroy, delete, room end or the parent's deadline cascade depth-first, as a durable effect with a startup recovery pass (T3 idea 2). This reverses "Deletion never silently destroys descendants" (`runtime/state/room_agent_admission.rs:187-197`), written for visible workers.

**Remote edge.**
- The home kernel owns the whole tree. A remote sub-agent is a *leased* agent: `CreateExecutionLease { home_kernel_id, home_session_id, home_agent_id, owner_user_id }` then `SpawnLeasedAgent` (`transport/relay_peer.rs:912-935`, `execution_lease.rs:10-20`). The lease already names the home agent; it gains the parent identity. Decisions return as projected interactions (worker lifetime already modelled, `pending_runtime_state.rs:93-99`) and a leased agent's runtime tools are forwarded home (`relay_peer.rs:1090`). So a remote sub-agent routes decisions, honours the ceiling and ends with its parent like a local one; the worker decides nothing about lineage.
- **Authentication over the peer link.** Today `LeaseCallerBinding` (`transport/relay_client/peer_requests.rs:2255-2330`) needs a machine identity, the sender key thumbprint and a match to the worker's *Cloud enrollment* (realm and user) or to one selected home. That is Cloud-account identity and conflicts with decision 6. New: the worker keeps a user-approved list of `(home kernel id, relay key thumbprint)` and accepts a lease only from a pinned key; realm and user checks go. Cloud only lists kernels.
- **Remote spawn approval.** A kernel-authored decision on the home kernel names the target kernel and the user approves it (`ApproveRemoteMachine` is machine-level and human-only, `local/api/types/remote_access.rs:12`). OPEN Q2: one card per spawn, or per tree?

**Protocol shapes.** Numbers come only through `COORDINATION_REQUEST.md`; notifext plans local 1000 and ghowner may need one, so mine follow them. Local: `parent_agent_id`, `SetAgentParent`, `ListDescendants` (S2); `EventResolutionVia::ParentAgent` and a `route` on projected interactions (S5); `SendAgentMessage`, `RegisterAgentEvents` (S6). Relay peer, one bump (S7): `DeliverAgentMessage`, `SubscribeAgentEvents`, `DeliverAgentEvent`, shaped like `SubscribeWorkflowNotifications` / `DeliverWorkflowNotification` (`transport/relay_peer.rs:830-839`), and a parent identity on `CreateExecutionLease`. Snapshot and hash tests change each time; client minimum versions stay.

## 3. Decision routing (b)

### 3.1 What exists
- A decision is a `RuntimeInteraction` (kind `Permission` or `Choice`, `session/runtime_interactions.rs:239-285`) about an agent or a kernel operation, held as a `PendingInteraction` with a one-shot `responder` (`runtime/state/pending_runtime_state.rs:70-89`).
- `RespondToInteraction` runs `answer_terminal_runtime_interaction` (`runtime/state/runtime_interaction_state.rs:326`), which already refuses owner-only kinds to non-terminal callers (`:337-397`). The first answer wins under the interaction lock, the loser gets "no longer pending" (`runtime_interaction_owned_state.rs:553-600`), and the answerer and an outside-event `via` are recorded (`:178`, `:578`).
- In orchdrill the orchestrator saw its worker's decision as a record (`value.question`) and was refused: `reach_tools.rs:41-58,202-207` and `sudo_request_forbidden` (`runtime/state/sudo/policy.rs:225`).
- A blocking card ends itself after 280 s (`tool_dispatch/popup.rs:8`); a question carries `timeout_sec` / `default_on_timeout`.

### 3.2 Decision — routing is a kernel field; delivery reuses the inbox
- When a sub-agent raises a decision the kernel computes `route`: the nearest ancestor that *can* answer, else the user. `route` lives on the pending interaction, not in any prompt.
- The parent receives the decision in its durable inbox through the wake and obligation machinery that already reports a finished child (`agent_inbox_delivery.rs`, `agent_wakes.rs`). A parent always follows its children (no `no_subscribe`). **Park and resume:** the parent waits parked on `events.yield` covering its child obligations; the kernel resumes it. A *decision* is delivered as a **steer** (it reaches a running parent turn now, or wakes a parked one, because a child is blocked on it); a *child finished* wake is **queued** (section 8, S10). Steers are never delayed or merged.
- The parent answers with the same `RespondToInteraction`. The kernel allows it only when the caller is the current `route` of that interaction. This replaces the blanket `answers_own_room` refusal: an agent never answers its own decisions, and answers only descendants routed to it.
- A parent that cannot or will not answer calls `PassDecisionUp`; `route` moves to the next ancestor or the user.

### 3.3 Hop by hop
1. **Raise.** The kernel classifies the decision (3.4). User-only kinds go straight to the user; others to the parent.
2. **Skip rule.** A hop is skipped with no model turn when that ancestor *could not approve it for itself*: its permission level is `Required` and would itself ask the user for that tool (section 4: a parent passes on only authority it holds). Also skipped when the ancestor is ended, archived, past its deadline, errored, or blocked on a user decision.
3. **Hop timer.** A hop lasts at most 120 s (OPEN Q3): one kernel timer per decision, started when the event reaches the parent. A busy parent does not extend it. It sits beside `kernel_operation_deadline` (`pending_runtime_state.rs:84`): no new lock, no unbounded wait, no polling. At expiry the route moves up.
4. **User.** The last hop is the user's client, in the top parent's panel. The user may answer at any hop; that cancels the pending hop event. The question's own `timeout_sec` / `default_on_timeout` keeps running across hops.
5. **Parent ends, errors or is cancelled mid-hop:** the route moves up at once. A decision never waits on a dead agent.

### 3.4 Decisions that never leave the user (kernel-authored list)
Computed once at raise time from fields that already exist; the default is *deny*: a decision is delegable only if its subject is an agent of the subtree, its kind is `Permission` or `Choice`, and it has none of these marks.

| # | Kind | Existing marker |
|---|---|---|
| 1 | Secrets / Vault entry, credential prompts | `terminal_credential_owner` (`pending_runtime_state.rs:83`), `InputKind::Secret` (`runtime_interactions.rs:192`), Vault requests (`kernel_access/policy.rs:290-299`) |
| 2 | /sudo | `sudo:` operation ids and `RequestKernelSudo` (`critical_approval_passkey.rs:537`, `sudo/policy.rs:221`) |
| 3 | Passkey-gated approvals | `passkey_gate` (`runtime_interaction_owned_state.rs:79`, `critical_approval_passkey.rs:510`); terminal class only (`:498`) |
| 4 | Payments | payment-card value kind, one owner decision per use (`runtime/state/vault_use.rs`, `config/vault_use.rs`) |
| 5 | Sign-ins and credentials: provider login, pairing, relay and Cloud identity, browser human verification | `provider_login_is_human_only` (`runtime_interactions.rs:622`), `handoff-` ids (`runtime_interaction_state.rs:381`), `policy.rs:303-321` |
| 6 | Renting or starting a machine | `kernel_operation_owner` / `capability-request-` (`runtime_interaction_state.rs:337-350`) |
| 7 | Kernel access grants | `access-grant:` / `access-extension:` and `requester` (`critical_approval_passkey.rs:535`) |
| 8 | User-domain browser decisions | `user_domain_interaction` (`runtime_interaction_state.rs:365-378`) |
| 9 | App/tool grants above the asker's parent | "owner approval or an explicit held subset" (`app_bindings.rs:181`) |

### 3.5 Audit and races
- `EventResolutionVia` gains `ParentAgent`; the resolution records the path `[child, ..., answerer]`. History shows "answered by <alias>" or "answered by you" in kernel text.
- Parent and user answering together is one race on the existing lock: the first valid answer wins. A late agent gets a typed "settled by user" result; a late click is dropped by the card; the user sees no error. The ceiling is rechecked on the answer (section 4), so an answer cannot raise authority.

### 3.6 What the user sees (data only; UI needs the owner's mock)
The top parent's panel gets a kernel-authored projection: each live descendant `{alias, depth, state, placement, open decisions}` plus the decisions routed to the user, labelled with the child alias. The prototype has no "show on demand" toggle or escalated card, so both need a mock in `PROPOSALS.md` before any UI (S9).

## 4. The ceiling (c)

**Rule.** The kernel stores what the child *asked for* and computes `effective(agent) = min(own, effective(parent))` at the moment of use, so a user who lowers a parent, or a grant that expires, constrains the whole subtree at once (T3 idea 4). Spawn refuses a request above the parent's effective values.

**Check under the lock.** The check is repeated for every control command and every decision answer, in the same serialized unit as the action, so nothing changes between check and effect: room-actor commands already run one at a time (`runtime/session_actor.rs:18-72`, which lists `RespondToInteraction`, `UpdateAgentConfig`, `SpawnAgent`, `DestroyAgent`); the answer also holds the interaction lock (3.5). Control commands outside the actor today (deadline, cancel) join it in S3/S4. The user can change a mode mid-flight; the next check sees it. **Remote carrying rule:** every home-to-worker request (`SubmitLeasedPrompt`, `SteerLeasedPrompt`, `CancelLeasedPrompt`, `UpdateLeasedAgentConfig`, `relay_peer.rs:943-1081`) carries the parent's current effective limit, not only the spawn (`SpawnLeasedAgent` carries mode and level once, `:927-930`). The worker applies the same minimum and refuses a request above it. **Not a comparison for control:** whether a parent may *control* a child is subtree membership (section 5); the limit only bounds what the child may *do*, so the user raising a child never locks its parent out (T3 issue 30408).

| Dimension | In the code today | Rule child ≤ parent | At spawn / at decision |
|---|---|---|---|
| Permission level (`Required` < `Yolo`) and mode (`Plan` < `Build`) | `provider/launch_contract.rs:601,636`; room spawn copies the parent's override (`meta_runtime_command/request.rs:44`), but `SpawnAgent` under /sudo can set any | level and mode ≤ parent's | spawn refuses higher; a `Required` parent cannot approve (3.3) |
| Tool and App allowance | `ExtensionGrant` (`extension.rs:280`); App delegation already carves from the parent (`app_bindings.rs:249-272,349-352`) | grants ⊆ parent's, safety ≤ | generalise the App rule to every kind; recheck at call |
| Workspace / worktree / write access | `worktree_id`, `workspace_id`, `ProviderWriteAccessMode` (`launch_contract.rs:671`); room spawn takes `--dir`, `--worktree`, `--from` | the parent's worktree, or a new worktree cut from the parent's repository under its workspace root | spawn resolves the path and refuses one outside (to verify in S3) |
| Network and sandbox | no separate switch: yolo means network on (`codex_client/permission.rs:307`) | inherited through the level | none new |
| Machine and rent rights | `MoveAgentToRemote`, `ControlPlane` acquire (`room_agent_admission.rs:127-148`) | placed or renting only where the parent may | spawn checks placement; renting stays a user decision (3.4 row 6) |
| Account and profile | `account_profile`, `resolve_spawn_account` (`runtime/agent_account_admission.rs`); room spawn passes none | profile ∈ the parent's allowance (its own plus those the user allowed it); the provider is part of the profile | spawn refuses outside |
| Deadline | `AgentDeadline`; shorten only (`docs/AGENT_DEADLINES.md`) | child deadline ≤ parent's | spawn clamps; parent's deadline ends the subtree |
| Usage | only observed: context tokens, account windows (`local/api/types/usage_snapshot.rs`) | **no token budget in v1** (OPEN Q4); proxies: depth ≤ 4, ≤ 16 children per parent, ≤ 64 per tree, shared deadline | spawn refuses over the caps |

Spawning (also by a child, in its own live turn) and Vault handles follow the same rule: a child gets no handle its parent does not hold. Decision-time enforcement happens in three places: a child's tool admission (`room_agent_admission.rs`, the same fence that checks the actor today), the answer to a routed decision (3.3), and launching a provider run (the level and mode sent to the run, `SpawnLeasedAgent` for remote).

## 5. Parent control without /sudo (d)

**One rule replaces `direct_child`:** a caller may control a target when `in_subtree(actor, target)` holds (any depth; a remote target reaches the same code through the lease). It is membership, never a comparison of modes (section 4). It covers deadline set/get, cancel the active turn, steer, stop/delete, usage and profile reads, spawn options inside the allowance, and reading the child's files and history.

| Orchdrill gap | Closed by the tree | Separate fix |
|---|---|---|
| ORCH-04 answer a worker's decision | section 3 | none |
| ORCH-05 deadline set/get | `in_subtree` + room command `agent deadline` (`SetAgentDeadline` is admitted for direct children, `room_agent_admission.rs:107`, but is no room command) | none |
| ORCH-06 `deadline_reached` in the in-room completion | no | typed reason in the completion payload (S1) |
| ORCH-07 cancel a turn without deleting | `agent cancel` | none |
| ORCH-08 usage and profile reads | `agent usage`, `agent profiles` | none |
| ORCH-09 spawn with profile and permission | `agent spawn --account-profile --permission --deadline`, clamped by section 4 | none |
| ORCH-10 hand over files | kernel-served `agent files` and `agent history` on a descendant (local, or through the lease); no attachment | none |
| ORCH-11 failed spawn leaves a stuck obligation | no | a remote spawn failure is always reported "uncertain" (`app/kernel_session.rs:322-330`, `dispatch_error(.., None)`) so the obligation never settles; a definite refusal must settle as rejected; spawn takes a command id so a retry returns the same child (S1) |
| ORCH-14 / 15 restart read as a user refusal | no | a restart reaches the worker as `InterruptedByRestart` (`provider/turn_outcome.rs:22`), not "The user doesn't want to proceed" (`app/provider_output_claude_native/transcript.rs:348`) (S1) |
| ORCH-16 typed outcome | no | `TurnOutcome` in the in-room completion (S1) |

A sub-agent ends with its parent (section 2); only `SetAgentParent` orphans it. Control survives the user raising a child above its parent: rights come from membership.

## 6. Peers and the reshaped reach (e)

**Three relations, one level above.** Assistants and external agents sit exactly one level above room agents: they act for the user as a client or terminal, within a grant (`docs/EXTERNAL_AGENTS_V2_PLAN.md`), and are not agents in the tree.

| Relation | Authority | Can do |
|---|---|---|
| User, clients, external agents, assistants | the user's, within the grant; the human-only set and the nine user-only decisions stay with the human | address *any* agent, including sub-agents by id: prompt, steer, cancel, read, answer routine decisions |
| Parent to its sub-agents | delegated, carved from the parent's (sections 4-5) | control the subtree, answer routed decisions |
| Peers (not ancestor and descendant, any room or kernel) | none | message, register to events |

An assistant is neither a descendant nor a peer. Like an external agent it sees sub-agents (agent list with lineage, `ListDescendants`, event targets by agent id: `validate_event_targets` checks room membership and a sub-agent lives in its top parent's room). It does not bypass the tree: a decision still routes to the parent first, and an assistant's answer is one more first-answer-wins answer, audited "answered by assistant <name>" (3.5).

**Today.** A Room agent of the kernel owner acts *as the owner* elsewhere through `chariox-reach` (`runtime/router/reach_tools.rs`, `runtime/state/kernel_access/reach.rs:36-95`): attach, `SubmitPrompt`, `RunShellCommand`, files, `ControlPlane`, `RespondToInteraction`, events; the rest needs /sudo, minus an exclusion list. Its attachments are `AutomationOnly` (`runtime/state/attachment_owned_state.rs:228`), which breaks its file reads (orchdrill).

**Decision 6.1 — peers have two primitives and no authority.**
1. **Message.** `SendAgentMessage { to: {kernel?, room?, agent}, text, command_id }`. The kernel stamps the sender `{kernel, room, agent}`. The receiver gets an agent-authored inbox event, never a user prompt (a `SubmitPrompt` through reach is a user prompt today). `command_id` makes a retry one delivery (`S5-RETRY` behaviour).
2. **Register to events.** `RegisterAgentEvents { agent addresses }` follows turn end, state change and errors of the named agents, with the cursor and wait of `chariox.events.*`. The sender of a spawn in another room is registered automatically unless it opts out (today's auto-follow). Decision events are *not* delivered to peers.
- Both work the same in the same room, another room, and another kernel. Across kernels they travel the relay peer link as `DeliverAgentMessage` / `SubscribeAgentEvents` / `DeliverAgentEvent`.
- Peers cannot answer each other's decisions, control, read files, run in each other's shell or terminal, read history, or act as the user.

**Cross-room spawn.** An agent that spawns in another room on the same kernel creates an ordinary agent of that room (a peer). Its authority is clamped to the spawner's at creation (frozen), so spawning cannot raise power. The spawner cannot control it and is registered to its events (OPEN Q5).

**Cross-kernel.** The peer link authenticates the *kernel* (key pinned by the user, section 2); the receiver trusts that kernel's claim of which agent sends. Messages and registrations carry no authority; a remote *spawn* needs the user's card (OPEN Q2).

**Retired:** the generic reach `request`; `ReachNeed::{Room, Sudo}`, `reach_sudo_turn`, `answers_own_room`; agent-made `AttachToSession` for reach (`AutomationOnly` stays for triggers and workflows, `agent_trigger_runtime_state.rs:893`); the `reach:<run>` owner identity. Acting as the user moves to external agents and assistants, which already have it (EA2). `/sudo` stays as the user's grant for the agent's own admin actions at home and never reaches a subtree or a peer. `RespondToInteraction` is allowed to agents only through routing (3.2). Human-only requests become a surface test: no tool carries them.

**The 29 agent-reach rows** (keep / re-scope / retire):

| Row | Disposition |
|---|---|
| S2-01 tools generated from the CLI table | re-scope: set shrinks to message, register, kernels, guide |
| S2-02 / 03 / 04 provider tool views (Claude, Codex, OpenCode) | re-scope: re-photograph the new set |
| S2-05 kernel argument refused on tools that do not declare it | keep (applies to the address argument) |
| S2-06 events tools before S3 | retire (superseded) |
| S3-01 follow an agent on another kernel | re-scope: agent-event registration over the peer link |
| S3-02 read another room's history | retire for agents (EA2 covers external agents and assistants; OPEN Q6) |
| S3-03 answer another room's routine decision | retire (external-agent twin EA2; agents use routing rows) |
| S4-01 agent acquires a machine | re-scope: own room tool; the user decides |
| S4-02 ask before agents acquire machines | keep |
| S4-03 Machines view lists the agent machine | keep |
| S4-WORKING-AGENT-COUNT | re-scope: tree counts roll up to the top parent only |
| S5-NEWROOMS create a room, local or remote | re-scope: peer creation, auto-register; remote needs the card |
| S5-EA2-10 human-only refusals | retire (EA2-10 itself stays); surface test that no peer tool carries them |
| S5-EA2-11 sudo-class refused without /sudo | retire (no passthrough; EA2-11 stays) |
| S5-EA2-15 work surface in another room | retire (peers have no shell; EA2-15 stays); parent reads child files instead |
| S5-EA2-16 working day (NOT RUN) | retire (external-agent area) |
| S5-EA2-26 follow wakes on turn end and decision | re-scope: turn end only |
| S5-EA2-27 only followed targets wake | keep |
| S5-EA2-31 one wait for local and remote | keep |
| S5-GRANTS-NOT-INHERITED | keep, add: child grants ⊆ parent's |
| S5-SETTING ask-before-acquire menu | keep |
| S5-GUEST (NOT RUN) | keep: a guest's agent has no peer reach |
| S5-RETRY lost reply, one effect | re-scope: applies to `SendAgentMessage` |
| S5-SUDO-ACQUISITION | keep (own room tool, not reach) |
| S5-GATEWAY-REVOKED-RUN | re-scope: user unpairs a kernel or revokes peer reach |
| S5-ELEVATED-SPAWNS | re-scope: sub-agent spawn always follows; cross-room unless opted out |
| S5-PRE-ENROLLMENT | retire (no agent cross-room reads; development-stage policy) |

Migration: no backward compatibility. `chariox-reach`, `reach_tools.json`, the "Agent reach" part of `docs/KERNEL_ACCESS.md` and `scripts/drills/agent-reach.mjs` are replaced or deleted in one slice (S8).

## 7. Provider-native sub-agents (f)

**Today.** The kernel leaves them to the provider. Claude events with a `parent_tool_use_id` are not the main stream (`provider/claude_runtime/events.rs:67,271`) and their usage folds into result totals (`:316`). Native sub-agents are disabled only in special modes: Codex `multi_agent` in metadata-only discovery (`provider/codex_client/thread_runtime.rs:418`), OpenCode `task` when native writes are off (`opencode_client/prompt_request.rs:79`, its `task` permission is `Critical`, `opencode_runtime/permission.rs:97`).
**Recommendation: ignore for lineage; keep them inside one Chariox agent.**
- They run in the parent's provider process with the parent's permission level, so they cannot exceed it; their permission requests come through the parent's run and route up the tree like the parent's own.
- Mapping them to edges would add children that cannot be steered, ceilinged or addressed (T3 shows them read-only too).
- Add one line to the agent instructions (T3 idea): native for small same-provider, same-worktree work inside one turn; a Chariox sub-agent for another provider or profile, a separate worktree, background work, a remote machine, or anything the parent must cancel, steer or give a deadline.

## 8. What we borrow from T3 Code (g)

Source: `T3_FINDINGS.md` (pingdotgg/t3code @ 6577c6b61, MIT, "Copyright (c) 2026 T3 Tools Inc."). We borrow ideas, not code, so no notice is needed. Copied code or text must keep their copyright line and MIT text in the file or our NOTICE. Never copy from their native, mobile, office-viewer, QR or `.repos` directories (other licences).

- Idempotent spawn by client request id, spawn contract, worktree isolation → S1 (fixes ORCH-11), S4.
- Completion cohort (one wake for all child-finished events ready) → S10, **queued wakes only**: steers (decisions, urgent messages) are never delayed or merged. Orchdrill shows one model turn per event.
- Question escalation, extended to approvals (T3 does not route approvals) → section 3. Limit rechecked at use and carried across environments → section 4, S7. Control = subtree membership, separate from the ceiling → section 5. Kernel-stamped sender provenance → section 6. "Completion wakes you, do not poll" in the running-task text → S4.
- Aggregate flag for a descendant needing a decision → the top parent's panel only (3.6).

We do not copy T3's polling (`t3_thread_wait`, follower long-polls): every parent, child and peer interaction here is a kernel event or obligation, and waiting is park and resume. T3 has no depth or fan-out cap, no usage budget, no orphan feature and no registration between peers; those are our own design.

## 9. Slices (h)

Details, red-first tests, drills, tracker rows and lane overlap are in `SUBAGENTS_SLICES.md`. **S1** independent fixes (M, none); **S2** lineage, hidden projection, lifecycle (L, local bump); **S3** ceiling (M); **S4** parent control surface (M); **S5** decision routing (L, local bump); **S6** peers on one kernel (M); **S7** cross-kernel, pinned keys, remote card (L, peer bump); **S8** retire old reach (M); **S9** top parent's panel (M, owner's mock first); **S10** batch queued wakes (S).

## 10. Open questions for the owner

1. **Caps:** depth 4, 16 children per parent, 64 per tree (T3 has none). OK?
2. **Remote approval:** one card per remote spawn, per tree, or pairing only? Is pairing enough for peer messages and events across kernels, given "paired kernels do not inherit grants"?
3. **Hop timer:** 120 s per hop before a decision moves up. Right length?
4. **Usage budget:** the kernel only observes usage; v1 has no token budget (deadline and caps stand in). Want a hard stop, for example a share of the account window?
5. **Cross-room spawn:** an agent that spawns in another room creates a peer it cannot control. Confirm, or make it a sub-agent hosted in that room's kernel.
6. **Peer history reads:** peers lose reading each other's history. Want a read-only outline back?
7. **Existing workers:** after S2, workers an agent spawns in a room leave the room list and appear only under their parent, Meta-mode helpers included. Confirm.
8. **Delivery class:** decisions to a parent as steers, child-finished as queued wakes. Confirm.
9. **Assistants and sub-agents:** a room-level follow by an assistant reports top agents plus a rolled-up "needs decision" for each tree; a sub-agent is followed by its own id. Confirm.
10. **UI:** the "show sub-agents on demand" affordance and the escalated card are not in the prototype. Which mock do we build?
11. **Conflicts with your decisions, flagged not reopened:** (a) no enforceable usage budget exists (decision 3); (b) the lease path authenticates by Cloud realm and user today (decision 6), so S7 removes it; (c) a peer created in another room cannot be controlled by its spawner (decisions 4, 5).
