# PEERS_P5: peers across kernels, slice plan (Landing 6, M2)

Design: `sub-agents.md` 7.5. This file holds the wire shapes, the numbers that move, the slices, the tracker rows and the two-kernel drill. Code references are at `d1891c4b3c` (peers3 head) unless they name peers1 (`b837f5fc42`) or peers2 (`ff553df9cf`). Nothing here is built.

## 1. What exists and what P5 reuses

| Piece | Where | Use in P5 |
|---|---|---|
| Same-owner peer seam: sender binding, owner from the authenticated identity | `transport/relay_client/peer_requests.rs:350-438`, `require_bound_managed_context_sender` (`relay_client/sender_identity.rs:125`), `same_owner_peer_user_id` (`runtime/state/workflow_notification_peers.rs:59`) | carries every P5 request |
| Sender call | `send_same_owner_peer_request` (`workflow_notification_peers.rs:107-140`), 3 s notification timeout (`:11`) | A -> B and B -> A sends; P5 uses its own timeouts (section 3) |
| Owner needs a Cloud profile | `authenticated_owner_of` (`workflow_notification_peers.rs:77-91`) | self-hosted pairs refused today (open question 3) |
| Kernel selector | `select_remote_kernel_by_ref_with_config` (`app/remote_agent_binding.rs:1464`), `normalize_local_kernel_ref` (`runtime/state/session_lifecycle_runtime_state.rs:636`) | resolves `--kernel` / `--machine` and the tool's `kernel` |
| Generation fence for subscriptions | `SubscribeWorkflowNotifications.generation` (`transport/relay_peer.rs:832-845`) | `SubscribePeerAgent` |
| Durable retry outbox, `accepted` / `retryable` | `durable_state/workflow_notifications.rs:416-440` (rows joined to App automations) | model for `peer_agent_outbox` (section 3) |
| Cross-room message, reply, steer, cancel; card fence | peers1 `runtime/state/peer_rooms.rs:15,58,89-103`; `agent_lifecycle_runtime.rs:524-561`; `durable_state/agent_lifecycle/replies.rs:40-68`; `runtime/router/reach_tools.rs:43,189` | B applies the same code to a remote caller |
| `peer_event`, one-wake merge | peers2 `durable_state/agent_lifecycle/transitions.rs:720-734`, `agent_lifecycle.rs:1067-1080`, `Registration.peer` (`types.rs:97-108`) | A applies a remote outcome through the same path |
| `agent spawn --room`, clamp, no edge | peers3 `meta_runtime_command/spawn_args.rs:245,396-414`, `dispatch.rs:691-700`, `room_agent_admission.rs:222-253`, `runtime_state_views.rs:180`, `session_actor/store/agent.rs:20,284-318` | B creates the peer with the same code; A lifts the `--kernel` refusal at `spawn_args.rs:402` when `--room` is given |
| Today's `agent spawn --kernel` | execution lease (`remote_agent_binding.rs:713-1060`, `CreateExecutionLease`, `SpawnLeasedAgent`, `relay_peer.rs:919,930`) | not used by P5 |

## 2. Agent-facing changes (no protocol number)

- `chariox.send_agent_message`: `SendAgentMessageArgs` (peers1 `transport/runtime_tools/agent_messaging_tools.rs:6-18`) gains `kernel: Option<String>` next to peers1's `room`. When `kernel` is set, `attachments` must be empty.
- `chariox.events.subscribe`: the peer address that peers2 adds gains `kernel`.
- Room commands: `agent cancel <ref> --room <room> --kernel <k>` (peers1 `meta_runtime_command/agent_tree.rs:113-160`); `agent spawn --room <room> --kernel <k> | --machine <m> [--no-follow]`. The refusal at `spawn_args.rs:402` stays for `--kernel` without `--room`; that is the sub-agent path.
- Tool results: `delivered`, `duplicate`, `uncertain` or a refusal reason (section 3). Each reason is rendered as one calm sentence by the tool layer on A.
- `SpawnAgentRequest` (`local/api/types/agent_lifecycle.rs:11`) does not change. The room command builds the peer request inside the kernel, so the local protocol stays 1000.

## 3. Wire shapes (relay peer protocol)

New file `apps/kernel/src/transport/relay_peer_agents.rs` holds the types. The variants join `RelayPeerRequest` (`relay_peer.rs:793`) and `RelayPeerResponse` (`:1266`). All P5 shapes land in P5a with **one** relay peer number, so P5b to P5e add behaviour only.

```rust
pub struct PeerAgentStamp { kernel_id: String, room_id: String, agent_id: String }   // filled by the sending kernel
pub struct PeerAgentAddress { room: String, agent: String }                          // id, ref or alias on the target kernel
pub struct PeerCallerAuthority { mode: AgentExecutionMode, permission_level: AgentPermissionLevel } // launch_contract.rs:601,650

// RelayPeerRequest, A -> B
PeerAgentMessage   { protocol_version: u32, request_id: String, sender: PeerAgentStamp, target: PeerAgentAddress,
                     message: String, urgent: bool, reply_requested: bool, caller_authority: PeerCallerAuthority }
PeerAgentCancel    { protocol_version: u32, request_id: String, sender: PeerAgentStamp, target: PeerAgentAddress,
                     caller_authority: PeerCallerAuthority }
SubscribePeerAgent { protocol_version: u32, request_id: String, sender: PeerAgentStamp, target: PeerAgentAddress,
                     subscription_id: String, generation: u64 }
UnsubscribePeerAgent { protocol_version: u32, subscription_id: String, generation: u64 }
SpawnRoomPeer      { protocol_version: u32, request_id: String, sender: PeerAgentStamp, room: String,
                     provider: String, account_profile: Option<String>, model: Option<String>, effort: Option<String>,
                     alias: Option<String>, prompt: Option<String>,
                     execution_mode: Option<AgentExecutionMode>, permission_level: Option<AgentPermissionLevel>,
                     caller_authority: PeerCallerAuthority, follow: Option<PeerFollow> }   // PeerFollow { subscription_id, generation }
// RelayPeerRequest, B -> A
DeliverPeerAgentOutcome { protocol_version: u32, delivery_id: String, outcome: PeerAgentOutcome }
pub enum PeerAgentOutcome {
    Event { subscription_id: String, source: PeerAgentStamp, occurrence_id: String, kind: PeerEventKind }, // turn_ended | turn_failed | source_lost
    Reply { request_id: String, source: PeerAgentStamp, occurrence_id: String, success: bool,
            public_answer: Option<serde_json::Value> },                                             // only for a message A sent
}

// RelayPeerResponse
PeerAgentAccepted   { protocol_version: u32, duplicate: bool, target: PeerAgentStamp }
PeerAgentSubscribed { protocol_version: u32, subscription_id: String, generation: u64, target: PeerAgentStamp }
RoomPeerSpawned     { protocol_version: u32, duplicate: bool, agent: PeerAgentStamp, alias: String }
PeerAgentOutcomeAck { protocol_version: u32, ack: OutcomeAck }          // accepted | duplicate | unknown_subscription
PeerAgentRefused    { protocol_version: u32, reason: PeerAgentRefusal }
pub enum PeerAgentRefusal { RoomUnavailable, AgentUnavailable, Ambiguous, MoreAuthority { mode, permission_level },
                            SubAgentCaller, RoomFull, AttachmentsLocalOnly }
```

- Reasons that never cross the wire are decided on A: `kernel_unreachable`, `uncertain` and `owner_unverified`. `RoomUnavailable` covers an unknown room, another owner's room and a hidden backing room alike, so B reveals nothing about the rooms of other owners.
- Admission on B: add the six variants to the `same_owner_request` match (`peer_requests.rs:350-356`) and to the `unreachable!` list (`:539-550`). Then refuse unless `sender.kernel_id == stable_peer_daemon_id(from_daemon_id)` (as "peer sender kernel mismatch", `:381-394`), and route to a new `router.relay_peer_agent(kernel, owner, request)` beside `relay_workflow_notification` (`runtime/router/relay_peer_bridge.rs:29`). Add trace labels in `transport/relay_client/peer_trace.rs:102`.
- Kernel modules, one job each: `runtime/state/peer_agent_outbound.rs` on A sends, maps `target_not_connected` / `target_disconnected` (`runtime/state/leased_task_connectivity.rs:264`) to `kernel_unreachable` or `uncertain`, and applies incoming outcomes through `Operation::SourceOutcome`. `runtime/state/peer_agent_inbound.rs` on B handles admission, dedupe and dispatch into peers1, peers2 and peers3 code.
- Durable state, one migration: on B, `peer_agent_requests (sender_kernel_id, request_id, response_json, expires_at_ms)` for dedupe, and `peer_agent_followers (subscription_id, subscriber_stamp, target_room_id, target_agent_id, generation, live)`. Also on B, `peer_agent_outbox (delivery_id, subscriber_kernel_id, outcome_json, state accepted|retryable|delivered|expired, next_attempt_at_ms, expires_at_ms)`. It is written in the same transaction as the event or reply that causes it. A dedicated table, because `app_outbox` rows are joined to App automations (`workflow_notifications.rs:435`).
- Timeouts: message, cancel and subscribe 10 s; spawn 60 s (`relay_request_timeout_ms` default, `config.rs:154`); outcome deadline 24 h.

## 4. Numbers and files to regenerate (not done in this round)

- `RELAY_PEER_PROTOCOL_VERSION` 439 -> **440** on today's base (`transport/relay_peer.rs:198`, with one doc line above it: "Peer 440 carries peers across kernels"). The coordinator allocates the number; another lane may take 440 first. `MINIMUM_RELAY_PEER_RUNTIME_VERSION` (`:201`) follows it, so both kernels must run the new build. Old kernels are not supported (development stage).
- `fixtures/protocol-versions.json` `relayPeer`; then `pnpm protocol:generate`. It rewrites `packages/kernel-client/src/kernel-types.ts`, `apps/kernel/managed-upgrade-protocol-transitions.json`, `docker/publication/Dockerfile` and the other files listed in `scripts/generate-protocol-versions.mjs`. Then `pnpm protocol:check`.
- New hash test `mod peer_agent_link_shapes` beside `workflow_notification_shapes` (`relay_peer.rs:2133`), with one instance of every variant and its SHA-256, and fixture `fixtures/peer-agent-link-protocol.json` (pattern: `fixtures/peer-browser-snapshot-protocol.json`). The version-pinned peer shape tests in `src/lib_tests.rs:75,191,289,411,437` and `relay_peer.rs:1641,1666,1778,2116,2336,2397` only re-read the pinned number, so their hashes should not change; run them. `pnpm protocol:conformance`.
- Local protocol: stays **1000**. No `LocalDaemonRequest` changes (section 2). Client minimums (`packages/kernel-client/src/*`, `apps/cli/src/kernel-feature-minimum.ts`): unchanged; web, TUI and iOS show nothing new.
- `docs/PROTOCOL.md`: the relay peer section gains the six requests and five responses.

## 5. Slices (one Sonnet lane; hours include fail-first tests and one live drill)

| Slice | Content | Fail-first tests (red on base) | Drill | Hours |
|---|---|---|---|---|
| **P5a** link, message, reply | all section 3 shapes and the number; admission and stamp check; owner check on both ends; dedupe table; `kernel` tool argument; message as `message` event with `sender_kernel_id` (fills `agent_inbox_read.rs:137`); reply through `peer_agent_outbox`; `kernel_unreachable`, `uncertain`, `attachments_local_only`, `owner_unverified` | a `PeerAgentMessage` whose stamp names another kernel is refused; a message lands once for two sends with one `request_id`; a reply on B becomes the `SourceOutcome` of A's task; hash snapshot | AO-PE-05 | 7 |
| **P5b** steer and cancel | `caller_authority` from `effective_agent_execution_config_in_tree`; B runs `authorize_peer_control` under the target room's actor; `MoreAuthority` refusal; cancel keeps the agent | Required caller steering a Yolo peer on B is refused; a Yolo caller's cancel stops B's turn and the agent stays | AO-PE-06 | 3 |
| **P5c** subscribe and `peer_event` | followers with a generation fence; `Event` outcomes for turn ended, turn failed, source lost; outbox retry after reconnect; `duplicate` ack; unpair or target delete -> `source_lost`; no decision events | three outcomes delivered twice give one inbox row and one wake; an older generation is ignored; a decision on B produces no outcome | AO-PE-07 | 5 |
| **P5d** `agent spawn --room --kernel` | lift `spawn_args.rs:402` with `--room`; `SpawnRoomPeer`; B reuses `authorize_peer_spawn` and `child_authority`; `max_agents` -> `RoomFull`; follow unless `--no-follow`; `--machine` | spawn into a missing room is `RoomUnavailable`; a sub-agent caller is `SubAgentCaller`; a retried `request_id` returns the same agent | AO-PE-08 | 4 |
| **P5e** offline, restart, fences | B stopped: `kernel_unreachable`; B restarted: outbox resumes; uncertain retry is one delivery; a variant-list test that no peer request reaches `RespondToInteraction`; card on B unanswerable from A | B down gives `kernel_unreachable`, not an error text; a card on B stays open after every peer request from A | AO-PE-09, AO-PE-10 | 3 |

Total about **22 h** (PEERS.md sized P5 at 16 h without the per-slice drills). Order: P5a -> P5b; P5a -> P5c -> P5d; P5e last. Two lanes can split P5b and P5c after P5a.

**Dependencies.** P5a and P5b need peers1 (message, reply, steer, cancel and the card fence). It is in the Landing 6 union `landing6/oss` (lane landunion6, head `4eb578710d` at 08:54Z: picks `93b5d7d496`, `c2773ce2c0`, `4671099c4d`, `4eb578710d`). P5d needs peers3, also in that union (`80062e753b`). P5c and the follow part of P5d need **peers2** (`013a947994`, `ff553df9cf`; its `tool_dispatch/peer_source.rs` was still uncommitted and unwired at 09:00Z). peers2 was not in the union at 08:54Z. P5 starts from the landunion6 head after peers2 is picked. P5 does not depend on slice RS (7.3, remote sub-agents in a hidden session), which is not built either.

## 6. Tracker rows to add (Peers, M2)

| Row | Promise | Evidence |
|---|---|---|
| AO-PE-05 | A room agent messages an agent in a room on another kernel of the same owner and gets its reply. The stamp's kernel is the authenticated link's; the event is a message, not a prompt; a retried request id is one delivery | two kernels, hosted |
| AO-PE-06 | A peer on another kernel steers or cancels an agent with no more authority than its own. A more privileged target is refused calmly and the target stays | two kernels, hosted |
| AO-PE-07 | Turn ended, turn failed and source lost of a peer on another kernel reach the subscriber once, also after a reconnect; decision events never cross | two kernels, hosted, B relay reconnect |
| AO-PE-08 | `agent spawn --room r --kernel k` creates an ordinary agent in room r on k: listed and counted there, no edge, clamped, followed unless `--no-follow`. A missing room and a sub-agent caller are refused | two kernels, hosted, web DPR1/DPR2 and TUI on k |
| AO-PE-09 | With the target kernel offline, every peer request is refused calmly with `kernel_unreachable`; outcomes already accepted arrive after it returns | two kernels, B stopped and restarted |
| AO-PE-10 | No card answer crosses the link: a card on k stays open whatever a peer on A sends | two kernels, hosted |

AO-PE-04 ("Peers have no control") is out of date since Miguel's 11-08 decision. It should read "Peers have no control beyond steer and cancel under the authority rule". The tracker owner should change it.

## 7. Two-kernel live drill recipe

- **Hosted (the acceptance path, needed while open question 3 keeps the Cloud owner).** Use the subval1 round-2 harness in `/root/.codex/evidence/subval1/r2/harness`. `/w/valenv/pr-stack up pr0-peers5-<slice> --repo /root/work/cloud.git --commit <cloud main> --oss-repo /root/work/oss.git --oss-commit <lane head> --relay-binary <unchanged relay>`. Then `kernel-up.mjs` (kernel A) and `kernel-up-b.mjs` (kernel B), both enrolled by device flow to the same fixture account, so both have the Cloud profile that `same_owner_peer_user_id` needs. Give the second kernel TMPDIR and XDG dirs under its own home. Create room R on A and room Q on B (`create-session.mjs`, `create-session2.mjs`). The room agent on A is Claude `haiku-5-5`, briefed with B's **daemon id** (not the machine alias) and Q's name. B's agent in Q is a second Haiku. Drive each row from A's agent. Read B with `chariox-cli --kernel-url ws://127.0.0.1:<B>/kernel` in tmux and the web room list at DPR 1 and 2. Capture `a.log`, `b.log`, `relay.log`, `steps.json` and screenshots in `/root/.codex/evidence/peers5/<round>/drills/<row>/`. Write the planned end into `/w/valenv/prs/<stack>/KEEP`; stop the stack after the round.
- **Local relay (quick red/green during a slice).** `scripts/drills/orchestrate-one-room.mjs --kernel-metadata yes --scenario remote` already starts kernels A and B on a local relay (`/root/.codex/evidence/subval1/r3/run.sh`; `{{B}}` in the brief is B's daemon id). A new `--scenario peers-remote` would add room Q on B and the rows above. **Not verified:** the driver forges relay tokens (`orchestrate-one-room.mjs:44`) and, as far as I read, sets no Cloud profile on the kernels. In that case `authenticated_owner_of` refuses, and the local stack can only show the `owner_unverified` refusal until open question 3 changes. The first P5a step is to run it once and record which.
- Red on base: on `d1891c4b3c`, `agent spawn --room Q --kernel <B>` is refused by `spawn_args.rs:402`, and `send_agent_message` has no `kernel` argument.

## 8. Not verified in this design round

No code was built or run. These were read, not run: the receiving owner check for self-hosted pairs, the local driver's Cloud profile, and whether peers2's subscribe stays one-shot (`live=false` after delivery) or becomes a lasting follow. P5c follows whatever peers2 lands. Line numbers in peers1 and peers2 are at their lane heads and will move when they reach the union.
