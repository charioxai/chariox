# Orchestration from Chariox: sub-agents, peers and the agent inbox

Status: design for the product owner, 2026-11-06, round 2 (MP-08 / MP-09 / MP-10 / MP-11). No product code, no UI mock. It folds in Miguel's answers of 11-06 15:23Z and the one-card steer of 15:36Z and the no-takeover decision of 15:48Z. Base: Landing 4 union head `cd799d2e60`, local protocol 999, relay peer 438. Code references are `apps/kernel/src/...:line` at that head. "(owner may overrule)" marks a reading the coordinator or I took.
Words: **top parent** = the room agent at the root of a tree; **sub-agent** = any descendant; **peer** = an agent that is neither ancestor nor descendant; **decision** = a permission or question card; **holder** = who must answer a decision right now.

## 1. What we build

The first milestone is **one room agent per room, plus sub-agents** (section 10). In ten lines:

1. One edge, `parent_agent_id`, makes a tree. A room agent spawning in its own room creates a sub-agent; a sub-agent can only create sub-agents.
2. Sub-agents are not in the room's agent list or agents view and not in `max_agents`. The user sees them in a folded stripe at the bottom of the top parent's panel, in the card it opens (a hierarchy), and in a sub-view of the room that looks like the agents view (section 8).
3. A sub-agent is an ordinary agent. A remote one is an ordinary agent on the other kernel: no lease, no tree on the home kernel (7.3).
4. Events reach an agent through its **inbox**: one very short prompt, then the agent reads the events (section 3). This ships first and helps every agent.
5. A sub-agent finishing or needing a decision **steers or wakes the parent at once**. No queued or timed wake.
6. A decision is **one card with one holder** that moves up the tree, parent to user. Nobody builds a second card (section 4).
7. A child never has more authority than its parent, checked at spawn and at every control command and answer under the action's lock (section 5). No usage budget.
8. A parent controls its whole subtree without /sudo (section 6).
9. Peers (any agent, any room, any kernel) can **message**, **reply** and **subscribe**; no control. Only room agents create peers (7.2).
10. Everything is event-driven: a parent parks on `events.yield` and the kernel resumes it. Nothing polls.

Assistants are not designed here; the model leaves room for them (7.4). **Removed since round 1:** the lease path, the remote-spawn card, the hop timer, the usage budget, App grants, the history outline, batched completion wakes.

## 2. Data model

**Today.** `spawned_by_agent_id` is an immutable creator receipt (`agent/types.rs:212`, `assert!` at `:339`); `direct_child` gives destructive rights over one level (`runtime/room_tool_admission.rs:29`); Meta mode has an older edge `controlled_by_metaagent_id` (`agent/types.rs:209`).

### 2.1 One edge, hidden from the room
- `parent_agent_id: Option<AgentRef>` replaces `spawned_by_agent_id` and `controlled_by_metaagent_id`. `AgentRef` is `{agent_id}` locally and `{kernel_id, agent_id}` when the parent lives on another kernel. Depth and root are derived by walking up.
- Spawn sets the edge once, from the spawner's current provider run (`runtime/state/room_agent_admission.rs:36-70`). Only a user-only `SetAgentParent { agent, parent | none }` changes it later ("promote", "orphan"). Agents cannot adopt or reparent.
- A local sub-agent keeps the top parent's room id (it needs a worktree, history and interactions) but is filtered from `ListAgents`, the archived list and the snapshot (`runtime/session_read_control.rs:237-246`) and from the counts (`agent/service.rs:97,192`). Requests by agent id (prompt, steer, cancel, state, streams) still work for hidden ids.
- Caps, all at spawn: depth 4, 16 children per parent, 64 per tree (the owner said ok).

### 2.2 Lifecycle
The edge is stored with the agent and in the `agent.created` event (`agent_lifecycle_owned_state.rs:34-45`), so it survives restart. A subtree ends with its parent: destroy, delete, room end or deadline cascade depth-first as a durable effect with a startup recovery pass (T3 idea). This reverses "Deletion never silently destroys descendants" (`room_agent_admission.rs:187-197`), written for visible workers. For a remote child the delete stays pending and retries until its kernel is reachable (7.3).

## 3. The agent inbox: a short wake, the events in the inbox

The owner asked: *the kernel prompts the agent with a very short message and the agent reads the message from its inbox. Do we have this shape?*

### 3.1 What exists, and what does not
- **Yes, the store and the tools.** Every agent has a durable inbox with sequence numbers (`InboxEvent`, `durable_state/agent_lifecycle/types.rs:107`). Tools: `chariox.events.inbox {after}` (up to 128 events), `ack {sequence, handled}`, `subscribe`, `yield`, `timer`, `blocked` (`tool_dispatch/agent_events.rs:133,205`).
- **No, the delivery.** `deliver_agent_inbox` takes the front event only (one per turn, `agent_lifecycle.rs:352-378`) and puts the **whole payload inline**: `"Kernel event inbox, untrusted data (sequence N, source S). ... {payload} ... Use chariox.events.ack ..."` (`agent_inbox_delivery.rs:187`). So agent-authored text rides inside a kernel prompt, prompts grow with the payload, and three events are three model turns (orchdrill: 0.8-2.7 M cache-read tokens per event).
- Only tools read the inbox (`agent_inbox(` is `#[cfg(test)]`), so no client protocol is involved.

### 3.2 Decision: one fixed short prompt (owner may overrule)
The prompt names counts and kinds only (a kernel enum), never agent-authored text:
- `Inbox: 3 new events (2 child finished, 1 decision) after sequence 37. Read them with chariox.events.inbox, then chariox.events.ack each one you handled.`
- Reply requested, always one message alone: `Inbox: 1 message with a reply requested (sequence 41). Read it with chariox.events.inbox after 40. Your final answer of this turn goes back to the sender; do not send a separate message.`
The "untrusted data" warning moves into the tool result and description.

### 3.3 What `chariox.events.inbox {after, limit}` returns
`limit` default 16, max 64; ordered by sequence; reading changes nothing; each payload is bounded to 16 KiB.
Each event: `sequence`, `kind`, `urgent`, `needs_answer`, `reply_requested`, `source {kind, id, alias, kernel_id}`, `summary` (kernel text, at most 160 characters), `payload`, `attachments`; the result also has `untrusted: true`, `next_after`, `more`.
Kinds this feature adds (ordinary `InboxEvent` rows):

| Kind | Source | Steers | `needs_answer` | Payload |
|---|---|---|---|---|
| `child_finished` | child | yes | no | `TurnOutcome` (with `deadline_reached`), answer reference |
| `child_decision` | child | yes | yes | interaction id, one-line question, choices |
| `message` | peer, kernel-stamped | sender's choice, default no | no | text, attachments |
| `peer_event` | followed peer | no | no | turn ended, state changed, error |

`timer`, `process`, `workflow_run`, `interaction_resolved` and the rest keep their kinds.

### 3.4 Acks, steer versus queue, one wake drains many
- **Ack** stays `ack {sequence, handled}` and gains `through: N` (all delivered events up to N that are not `needs_answer`). A `needs_answer` event cannot be acked until its decision is answered or passed. Acking an event before its wake was sent expires the wake without a prompt (`Expire`, `agent_inbox_delivery.rs:69-110`).
- **Steer versus queue is unchanged**, set by the event: urgent steers the running turn (`:154`); non-urgent waits for the turn to end; a parked agent starts a turn; a refused steer is deferred to the next turn. Child finished and child decision are urgent.
- **One wake drains many, without waiting.** The wake covers what is pending when it is built: a steer wake every pending urgent event, an idle or queued wake every pending event. No window, no timer. A later event gets its own wake, or is read and acked in the same turn. `Attempt` gains `through`; the batch rows share one prompt id and one delivery receipt.

### 3.5 The two fences in `agent_inbox_delivery.rs`
- **Correlated reply** (`replies.rs`; the `reply` text at `:182`): the reply is this turn's final answer, so a `reply_requested` message is delivered **alone**; its short prompt carries the reply instruction.
- **Sudo window** (`sudo_work_task`, `:48-53`; `WORK_CORRELATED`, `agent_lifecycle.rs:1052`): in sudo-bound work only events correlated to that task are delivered and readable (`agent_events.rs:136`); the rest stay deferred with the existing notice; nothing steers an active sudo turn. The batch holds correlated rows only; child events carry the parent's delegate `task_id`.
- Unchanged: a stopped agent keeps messages held (`:137`); the Vault-unlock wait (`:5`).

### 3.6 Protocol, shipping, tests
- **No client protocol change, no number, no relay change.** Changes: the tool schema of `chariox.events.inbox` and `ack` (three provider tool views, snapshot tests); a batch end on the durable row (`serde(default)`, no migration); the prompt text in room history.
- **Ships alone as slice A1, first.** It needs no tree and benefits every agent today (timers, processes, workflow runs, answers, messages). Size M. If A1 slips, later slices use the old prompt with the same events.
- Pinned tests that stay green (text expectations updated): `provider_output_runtime_tests/completion_settlement.rs`, `agent_reply_tests.rs`, `sudo/window_tests.rs`, `router/tests/agent_messaging.rs`, `agent_triggers.rs`, `local_prompt_dispatch_runtime/tests/inbox_recovery.rs`, `leased_wakes.rs`. New red-first tests and the drill: `SUBAGENTS_SLICES.md`.

## 4. Decisions: one card, one holder

### 4.1 The model (coordinator steer, owner may overrule)
A decision is **one interaction that belongs to the child** (`RuntimeInteraction`, `session/runtime_interactions.rs:239-285`; held as a `PendingInteraction`, `runtime/state/pending_runtime_state.rs:70-89`). It is never copied. The kernel only moves its **holder**: the child's parent, then that parent's parent, ..., then the user. Every place that shows the decision is a view of the same object, so an answer from anywhere settles it for everyone. The first valid answer wins under the interaction lock; the loser gets "no longer pending" (`runtime_interaction_owned_state.rs:553-600`).

### 4.2 Who sees what
1. **A parent holds it.** No user sees a card. The parent gets `child_decision` in its inbox as a steer (3.3). The child's pane in the sub-view shows a plain "waiting on <parent alias>" line with no buttons. The user's only lever is the one they already have: steer or stop the parent (and, by the cascade, its subtree); there is no timer and no takeover. The parent answers with the room command `agent answer <interaction> <choice | text>` or moves it on with `agent pass <interaction>`. Only the current holder may answer; agents never call `RespondToInteraction` directly. This replaces `answers_own_room` (`runtime/router/reach_tools.rs:42-58`).
2. **The user holds it** (a user-only kind, or the climb reached the top). The card appears in the **top parent's panel** and in the child's pane in the sub-view: same card, same id; answering either closes both. The top parent's stripe shows "needs you".
3. **A parent that wants the user's own words** asks its ordinary question with its own card (unlinked), then answers the child itself with the user's answer. This is the documented pattern; the kernel does not link the two.

### 4.3 Where a decision starts
- **Raise.** The kernel sets the holder when the card is created: the user for the eight user-only kinds (below); otherwise the nearest ancestor that could approve it **for itself**. An ancestor is skipped (the holder moves up at once, no model turn) when its permission level is `Required` and it would itself ask the user for that tool (a parent passes on only authority it holds, section 5), or when it is ended, archived, errored, stopped by the user, past its deadline or blocked on a user decision.
- **Holder ends mid-way.** If the holder ends, errors or is cancelled while holding, the holder moves up immediately and the former holder's inbox event is withdrawn. A decision never waits on a dead agent.
- **No hop timer.** The card's own `timeout_sec` / `default_on_timeout` still applies across holders.

**Decisions that never leave the user** (eight; default deny: delegable only if the subject is a subtree agent, the kind is `Permission` or `Choice`, and none of these marks applies):

1. Secrets, Vault entries, credential prompts (`terminal_credential_owner`, `pending_runtime_state.rs:83`; `InputKind::Secret`, `runtime_interactions.rs:192`).
2. /sudo (`sudo:` ids, `critical_approval_passkey.rs:537`).
3. Passkey-gated approvals (`passkey_gate`, `runtime_interaction_owned_state.rs:79`).
4. Payments (payment-card value kind, `runtime/state/vault_use.rs`).
5. Sign-ins: provider login, pairing, relay and Cloud identity, human verification (`provider_login_is_human_only`, `runtime_interactions.rs:622`; `handoff-` ids).
6. Renting or starting a machine (`kernel_operation_owner`, `runtime_interaction_state.rs:337-350`).
7. Kernel access grants (`access-grant:`, `critical_approval_passkey.rs:535`).
8. User-domain browser decisions (`user_domain_interaction`, `runtime_interaction_state.rs:365-378`).

There are no App grants in this design; every installed App is available to every agent. A card some code path may still raise for an App behaves as an ordinary permission card.

### 4.4 Races and audit
- Two answers at once (the holder moved to the user while the parent's answer was in flight, or an external agent with a grant): the first valid answer wins under the interaction lock. The late agent gets a typed "settled by user" tool result (`settled_by`); a late click is dropped by the card; nobody sees error text.
- The answer rechecks the ceiling under the interaction lock (section 5), so an answer cannot raise authority.
- `EventResolutionVia` gains `ParentAgent`; the projection records `holder`, `path` (holders passed, child first) and `settled_by` (user, agent, external agent, timeout).
- Remote child: the card lives on the child's kernel; the parent answers with `AnswerChildDecision` over the peer link (7.3) and the same lock decides.

## 5. The ceiling

**Rule.** The kernel stores what the child asked for and computes `effective(agent) = min(own, effective(parent))` at the moment of use, so lowering a parent constrains the whole subtree at once (T3 idea). Spawn refuses a request above it.

**Lock rule.** The check repeats for every control command and every decision answer, in the same serialized unit as the action, so nothing changes between check and effect: room-actor commands already run one at a time (`runtime/session_actor.rs:18-72`) and the answer also holds the interaction lock. Deadline and cancel join the actor (slice A5). **Remote carrying rule:** every request from a parent's kernel to a child's kernel carries the parent's current ceiling (level, mode, deadline, depth); the child's kernel applies the same minimum. **Control is not a comparison:** a parent controls a child by subtree membership (section 6); the ceiling only bounds what the child may do, so raising a child never locks its parent out (T3 issue 30408).

| Dimension | Today | Rule | Status |
|---|---|---|---|
| Permission level (`Required` < `Yolo`) and mode (`Plan` < `Build`) | `provider/launch_contract.rs:601,636` | child ≤ parent | **keep**: the core safety bound; also the base of the skip rule (4.3) |
| MCP, skill, script and connector grants and Vault handles | `ExtensionGrant` (`extension.rs:280`) | child's grants ⊆ parent's, safety ≤ | **keep**: otherwise a child could hold a credential its parent does not |
| Workspace and worktree | `worktree_id`, `workspace_id`, `ProviderWriteAccessMode` (`launch_contract.rs:671`) | a child's worktree lies inside the parent's workspace root | **keep**: file reach (verify in A5) |
| Deadline | `AgentDeadline`, shorten only (`docs/AGENT_DEADLINES.md`) | child ≤ parent; the parent's deadline ends the subtree | **keep**: makes the subtree end in time without a budget |
| Placement and renting | `MoveAgentToRemote`, `ControlPlane` acquire (`room_agent_admission.rs:127-148`) | placed only where the parent may; renting stays a user decision (4.3, row 6) | **keep** |
| Account profile | profiles belong to the owner (`runtime/agent_account_admission.rs:6`) | none | **drop** (owner may overrule): a profile is cost, not authority; with no usage budget there is nothing to bound |
| Usage | only observed | **none in v1**: the agent decides | dropped (owner: "nothing") |

Network and sandbox follow the level (`codex_client/permission.rs:307`). Caps (2.1) bound fan-out; App allowance is not a dimension (no grants).

## 6. Parent control without /sudo

**One rule replaces `direct_child`:** a caller may control a target when `in_subtree(actor, target)` holds, at any depth; a remote target reaches the same code through the peer link. It is membership, never a comparison of modes.

Room commands for the parent, each refused for a sibling, a peer, another room and a non-agent caller: `agent cancel <child>` (stop the active turn, keep the agent), `agent steer` (an urgent message to the child, the existing mechanism), `agent deadline set|get`, `agent delete` (the whole subtree), `agent spawn --account-profile --permission --deadline` (clamped by section 5), `agent answer` and `agent pass` (4.2), `agent children` (the subtree with state). Read-only `agent usage` and `agent profiles` are open to every room agent (owner may overrule). A parent reads a child's history with the ordinary history tools (7.4) and a local child's files with its own shell; only a remote child needs `agent files get|put` (M2).

Orchdrill gaps closed: ORCH-04 by section 4; ORCH-05 by `agent deadline` (`SetAgentDeadline` is admitted for direct children, `room_agent_admission.rs:107`, but is no room command); ORCH-06 and ORCH-16 by `TurnOutcome` and `deadline_reached` in `child_finished`; ORCH-07, 08, 09 by the commands above; ORCH-10 by own shell (local) or `agent files` (remote); ORCH-11 because a remote spawn failure is reported "uncertain" (`app/kernel_session.rs:322-330`, `dispatch_error(.., None)`) and never settles, so a definite refusal settles as rejected and spawn takes a command id (retry returns the same child); ORCH-15 because a restart must reach the child as `InterruptedByRestart` (`provider/turn_outcome.rs:22`), not "The user doesn't want to proceed" (`app/provider_output_claude_native/transcript.rs:348`). All in A2.

A parent **parks** on `events.yield` covering its children's obligations and is resumed by `child_finished` or `child_decision` (urgent, section 3).

## 7. Peers, remote sub-agents and the reshaped reach

### 7.1 Relations
- **User, clients, external agents (assistants later):** the user's authority within the grant; the human-only set and the eight user-only decisions stay human. They address any agent, including sub-agents by id.
- **Parent to its sub-agents:** delegated authority, bounded by the ceiling: control the subtree, answer the decisions it holds.
- **Peers** (not ancestor and descendant; same room, another room, another kernel): no authority. Message, reply, subscribe.

### 7.2 Peers
- **Message and reply.** `send_agent_message` (today same room only) gains an address `{kernel?, room?, agent}`. The kernel stamps the sender `{kernel, room, agent}`; the receiver gets an agent-authored `message` inbox event, never a user prompt. A reply uses the existing correlated-reply path (3.5). `command_id` makes a retry one delivery. `urgent` stays a delivery class of the message (owner may overrule).
- **Subscribe.** `chariox.events.subscribe` (today the source must be a live agent or workflow of the same room, `agent_events.rs:158-176`) accepts a peer address. The subscriber gets `peer_event` for turn end, state change and error. Decision events are not delivered to peers.
- **No control**: a peer cannot cancel, steer another's turn through a command, answer, read files, run in the other's shell, or act as the user.
- **Who creates peers.** Only room agents, with `agent spawn --room <room> [--kernel <k>]`: an ordinary agent of that room with no edge, authority clamped to the spawner's at creation, the spawner subscribed to its events unless `--no-follow`. A sub-agent's `--room` is refused: a sub-agent can only create sub-agents of its own.
- **Retired**: the reach `request` passthrough, `ReachNeed`, `reach_sudo_turn`, `answers_own_room`, the agent-made `AutomationOnly` attachment and the `reach:<run>` identity (`runtime/router/reach_tools.rs`, `kernel_access/reach.rs:36-95`). Acting as the user stays with external agents; `/sudo` never reaches a subtree or a peer.

### 7.3 Remote sub-agents (M2)
A remote sub-agent is **a normal agent on the other kernel**, started the way agent reach starts one today (`CreateSession` / `SpawnAgent` on the other kernel, `reach_tools.rs:3`), but with a narrow request instead of the owner passthrough.
- **Spawn.** `agent spawn --kernel B` makes the home kernel send `SpawnAgentWithParent { parent: {kernel A, agent X}, ceiling, command_id, ... }`. Kernel B creates an ordinary agent in a room of B (a new one by default) and checks the ceiling. No extra card for the spawn. On B it is a normal agent, listed in its room (owner may overrule: hide it there too). A shows it in the stripe and sub-view like a local one.
- **Edges and events.** B holds `parent_agent_id = {A, X}`. B sends A `child_finished` and `child_decision` events; A sends B `ControlChild` (cancel, steer, deadline, delete) and `AnswerChildDecision`. Each carries the ceiling (section 5) and a `command_id`. If B is unreachable, A shows the last known state and `unreachable`; cancel and delete stay pending and retry.
- **Authentication.** These are peer-link requests on the same-owner seam that workflow notifications already use (`same_owner_request`, `transport/relay_client/peer_requests.rs:350`), so one path serves Cloud and self-hosted, Cloud only lists kernels, and a paired kernel inherits no grants: the requests are narrow, agent-addressed and idempotent, never the owner passthrough. To verify in M2: a self-hosted pair passes `require_bound_managed_context_sender`; if not, the fix is in that shared seam.
- **Retired for sub-agents**: the execution lease, `SpawnLeasedAgent`, the Cloud realm and user check for them, and the pinned-key design of round 1.

### 7.4 History, assistants (one line each)
A sub-agent and a peer get the tools every room agent has (`chariox.history.search` / `read`, `runtime/state/public_history.rs`, secrets masked), bounded to the root room; nobody reads another room's history; no new mechanism. Assistants will sit one level above room agents with user authority inside their grant, neither descendants nor peers; nothing here changes for them.

## 8. What the user sees: the kernel projection (data only, not a mock)

**Gate.** The stripe, the hierarchical card and the sub-view need an **owner-approved mock** (PROPOSALS.md) before any UI slice. This section is the data they read.

`ListDescendants { agent }` (read) and the notification `AgentTreeChanged { root, rev }` (clients re-read; changes coalesced). Everything is enums and ids; the kernel adds no error text.

```
AgentTree { root: agent_id, rev, totals: { live, running, needs_user },
  nodes: [ { agent_id, parent_agent_id, kernel_id | null, alias, provider, model, depth,
             state: starting | running | waiting | idle | ended | error | unreachable,
             outcome: TurnOutcome kind | null, deadline_ms | null, created_at_ms,
             pending: [ { interaction_id, kind, holder: { kind: agent | user, agent_id? },
                          path: [agent_id...], settled_by: null } ] } ] }
```
- Order is creation order and stable; the client builds the hierarchy from `parent_agent_id`.
- **Stripe** reads `totals`. **Card** reads `nodes`. **Sub-view** shows one panel per node, using the same per-agent reads and streams as the room's agents view, addressed by `agent_id` (2.1). Clicking a card entry opens the sub-view if needed and focuses that node.
- A `pending` entry whose holder is an agent renders as "waiting on <alias>" without buttons. When the holder is the user, the card renders in the top parent's panel and in the node's panel (4.2). A settled entry reports `settled_by`.
- **Who may see it:** any client allowed to see the top parent's room and its panel (owner, and room guests as today); external agents within their grant. Peers never see another agent's tree; a parent sees its own subtree through `agent children`.
- Sub-agents are never listed in the room's agent list and never counted toward `max_agents`. A decision held by the user raises `needs_user` on the top parent's room entry (aggregate flag; T3 idea).

## 9. Provider-native sub-agents, and T3 borrowings

**Provider-native** (Claude Task, Codex `spawn_agent`, OpenCode `task`) stay inside their provider and are not edges: they run in the parent's process at its permission level, and their permission requests already come through the parent's run (`provider/claude_runtime/events.rs:67,271`). One instruction line: native for small same-provider, same-worktree work inside one turn; a Chariox sub-agent for another provider, a separate worktree, background or remote work, or anything the parent must cancel, steer or give a deadline.

**From T3 Code** (pingdotgg/t3code @ 6577c6b61, MIT, "Copyright (c) 2026 T3 Tools Inc."; ideas only, no code copied, so no notice is needed; copied code or text must keep the copyright line and MIT text; never copy their native, mobile, office-viewer, QR or `.repos` directories): idempotent spawn (A2), the stop cascade with recovery (2.2), question escalation extended to approvals (4), limit rechecked under the lock and carried across environments (5), control as membership apart from the ceiling (6), kernel-stamped sender (7.2), the aggregate flag (8). Not copied: T3's polling and (dropped) batched completion wakes.

## 10. The first milestone and the slices

**M1 = a room agent spawns a sub-agent, steers, reads and stops it, gets the finish wake and answers its decision card, with the user seeing it only in the stripe, the card and the sub-view.** Slice order (details, red-first tests, drills in `SUBAGENTS_SLICES.md`): **A1** inbox shape (M, none) -> **A2** tree edge, parent control, child-finished steer, spawn fixes (L, local number) -> **A3** one-card decision routing (L, local number) -> **A4** hidden projection and cascade (M, local number) -> **A5** ceiling at use (M) -> **mock gate** -> **A6** stripe and card, **A7** sub-view. **M2** peers and remote sub-agents (P1 peers on one kernel, P2 across kernels, P3 retire old reach). **M3** the rest.

## 11. Open questions for the owner (as few as possible)

1. **Remote sub-agent on its own kernel:** it is a normal listed agent in a room of that kernel (recommended: least new machinery), or hidden there too?
2. **Ceiling dimensions:** keep MCP/skill/script/connector grants (child ⊆ parent) and drop the account-profile dimension (recommended), or drop both because "everything is available to every agent"?
