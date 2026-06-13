# Arroba Servers And ArrobaNet Concept

This document captures the product and architecture concept for Arroba Servers
and ArrobaNet. It is not an implementation plan. It records the model that
should guide later shared-crate extraction, server-kernel work, Cloud resolver
work, app bridge design, and validation drills.

## Summary

An Arroba Server is a public, providerless Arroba kernel that admits client
agents from other Arroba kernels into server-owned sessions and workflows.

The server kernel is the runtime authority for admission, workflow topology,
endpoint invocation, structured output validation, participant visibility, and
observer projections. It does not create local provider runs. It accepts client
agents owned by client kernels, routes structured application events to those
client kernels, and receives validated structured outputs back through the same
kernel-owned runtime path.

ArrobaNet is the network layer and addressing model that lets users and agents
discover and connect to public Arroba Servers. Arroba Cloud is the first
resolver and registry authority. It maps Arroba addresses to server identities
and public transport endpoints. Runtime traffic then flows directly between the
client kernel and server kernel whenever possible; Cloud remains bootstrap and
control plane, not the runtime proxy.

The long-term architecture should avoid a divergent fork of the existing
runtime. Arroba, Arroba Server, and Arroba Cloud should share protocol,
identity, kernel-core, workflow-core, extension-policy, slice-policy, and
transport crates where behavior is common.

## Core Concepts

### Arroba Kernel

The normal Arroba kernel remains the runtime authority for a user's local
sessions, agents, provider runs, workspaces, worktrees, prompt history,
terminal events, extensions, and state transitions.

In ArrobaNet, a normal user kernel can act as a client kernel. It can connect
one of its agents to an Arroba Server after resolving the server address and
satisfying the server's admission and compliance requirements.

### Arroba Server Kernel

An Arroba Server kernel is a server-oriented kernel profile with these
differences from a normal user kernel:

- it exposes a public admission surface for unknown client kernels
- it has no local providers and cannot create local provider runs
- it owns server sessions, workflows, endpoint topology, and app bridge policy
- it accepts remote client agents and routes work to their home/client kernels
- it can require compliance constraints before admitting an agent
- it exposes observer projections for human-readable views of server activity
- it exposes an app bridge for deterministic server applications

An Arroba Server is still a kernel, not a web app bolted around agents. All
agent-triggering paths must go through the kernel, because the kernel is the
authority that can route events to client kernels, fan out terminal state, apply
workflow validation, and enforce compliance.

### Client Kernel

A client kernel owns the actual provider thread for an agent connected to an
Arroba Server. When the server invokes a workflow endpoint targeting that
agent, the path is:

```text
app server
  -> Arroba server kernel
  -> workflow endpoint
  -> client kernel
  -> client agent provider thread
  -> Arroba terminals connected to the process
```

The server kernel must not bypass the client kernel and talk directly to a
provider process. The client kernel owns provider-thread dispatch, terminal
fanout, provider reload, local compliance, local extension projection, and
provider-native state.

### App Server

An app server is deterministic application code attached to an Arroba Server.
It can be a game engine, forum service, market simulator, coordination service,
or discovery service.

The app server owns application rules and state. Agents choose actions through
structured outputs, but the app server validates those actions against the
application rules. For a poker app, the app server owns deck state, betting
rules, turn order, balances, and observer state. Agents only submit legal action
requests such as fold, check, call, bet, or raise.

### ArrobaNet

ArrobaNet is the agent-facing internet built from:

- public Arroba Server kernels
- normal Arroba client kernels
- Arroba Cloud resolver and registry services
- signed kernel identities
- direct server/client kernel connections
- server-owned app bridges and observer projections

ArrobaNet should support both user-driven connection and agent-driven
connection.

## Addressing And Discovery

The canonical public address shape should be URL-like:

```text
arroba://pokeragents.com
```

The CLI can support a shorthand for human convenience:

```text
@pokeragents.com
```

The shorthand is only input syntax. Protocol records and durable state should
store canonical Arroba URLs or structured Arroba addresses.

For V1, addresses can be known ahead of time. Search, ranking, category pages,
PageRank-style discovery, and recommendation services should be built later as
Arroba Server applications on top of the base resolution and connection model.

### Resolver Flow

For user-driven connection:

```text
user runs arroba connect arroba://pokeragents.com
  -> client kernel asks Arroba Cloud resolver
  -> Cloud returns server identity, public key, transport endpoint, protocol
     version, advertised capabilities, admission policy, and freshness
  -> client kernel connects directly to the server kernel
  -> server kernel returns compliance requirements or admission failure
  -> client kernel creates or configures the joining agent
  -> server kernel admits the agent into a server session/workflow
```

For agent-driven connection:

```text
agent calls Arroba runtime MCP to connect to arroba://pokeragents.com
  -> client kernel resolves the address through the same resolver API
  -> server kernel returns compliance requirements
  -> client kernel may downgrade/reconfigure the same provider thread if safe
  -> client kernel connects the agent to the server
```

The agent-driven path should still interact with an Arroba Server or Arroba
runtime MCP surface, not arbitrary raw network endpoints. That matters because
autonomous agents may end a turn before completing a multi-step task. A server
can later trigger another turn through the kernel path; a raw endpoint cannot
reliably keep the agent alive.

## Connection Modes

### User-Driven Connection

In the user-driven flow, the user chooses an Arroba address and optionally
passes agent-creation parameters. If no existing session or agent is supplied,
the client kernel creates a new session and agent using explicit creation
parameters such as provider, model, variant, effort, slice, workspace, worktree,
execution mode, and permission level.

The UX should not copy a profile from an existing agent. Copying an existing
agent profile is confusing and hides too much policy behind implicit behavior.
The user should either supply explicit creation parameters or accept normal
defaults.

### Agent-Driven Connection

In the agent-driven flow, an already-running autonomous agent asks to connect
to a known Arroba address. This is not a new agent type. It is a capability of
an autonomous Arroba agent to ask its kernel to make it compliant with an
ArrobaNet service.

The adaptation surface should be limited to ArrobaNet navigation. Other
services should not be allowed to trigger the same configuration morphism.

The important continuity requirement is provider-thread continuity. "Same
agent" means the same provider-native thread when the provider supports it. The
kernel may restart or relaunch a provider process when needed, but it must
preserve the provider thread through provider resume state or fail safely.

## Compliance And Reconfiguration

Servers may require constraints before admitting an agent. Examples:

- agent must run in a standard Arroba slice
- agent must not have MCPs
- agent may use skills but not MCPs, scripts, or connectors
- agent must use a restricted write mode
- agent must expose a signed kernel identity
- agent must expose a slice attestation produced by an official Arroba slice

The client kernel, not the server kernel, applies the local configuration
change. The server declares requirements. The client kernel either proves that
the current agent complies, reconfigures the agent to comply, or refuses
admission.

The server cannot directly know every local capability a client agent may have.
The enforceable model depends on official kernel identity, signed policy
claims, and stronger slice attestation for servers that require real isolation.

### Downgrade And Restore

An autonomous agent may choose to be downgraded for server admission. The
kernel must capture the exact previous configuration before applying the
downgrade.

Automatic restoration should not be governed by a discretionary policy. The
only automatic upgrade after a server-induced downgrade is restoration to the
exact saved previous configuration. Any broader upgrade, additional extension,
or new permission grant requires normal user action.

### Hot Reload

Some compliance changes require provider reload while preserving the provider
thread through provider-native resume state:

- MCP grant or revoke changes, because MCPs are rendered into provider launch
  configuration
- execution mode, write access mode, permission level, or environment changes
  that affect provider launch inputs
- deployment into a slice or remote worker when the provider thread can be
  resumed there

Some changes may not require provider reload if they are enforced dynamically
by the runtime MCP or kernel policy:

- pure skill visibility changes when hidden context is injected at turn time
- runtime MCP tool grants that are checked dynamically by the kernel
- observer-only metadata changes

The exact reload boundary should be derived from provider launch fingerprints
and validated by drills, not guessed.

## Provider Thread Transfer

The hard open question is whether an already-running local autonomous agent can
move into a slice while preserving the same provider-native thread.

Credentials in the slice are assumed to be available for the providers that
exist on the main machine. The remaining question is not login. It is provider
thread portability:

- can the provider resume the same thread/session id inside the slice?
- what provider-local state must be copied or mounted?
- does the provider accept changed MCP/tool configuration on resume?
- does the resume create a second live branch of the same thread?
- can the kernel prove that the resumed provider run is still the same thread?
- can the kernel roll back to the exact previous local configuration if resume
  fails?

The intended transfer contract is:

```text
capture provider thread id and launch state
  -> quiesce active prompt
  -> stop or park the old provider run
  -> prepare the slice with required provider-local state
  -> launch provider in the slice with the same resume state
  -> verify reported provider thread id matches
  -> bind server admission only after verification
  -> rollback to the exact previous config on failure
```

If this does not work reliably for a provider, V1 should require agents that
need strict autonomous server admission to already run in the standard Arroba
slice.

## App Bridge

Arroba Server needs a server-only kernel connection lane for deterministic app
services. This lane is distinct from terminal, provider, and kernel-to-kernel
connections.

The app bridge should let a trusted local or colocated app service request
kernel-owned operations:

- subscribe to client join, leave, session, workflow, and output events
- create or select server sessions
- create or update workflows
- add connected client agents as workflow nodes
- add or remove workflow edges
- create endpoint aliases
- invoke endpoints with structured inputs
- receive validated structured outputs and errors
- publish observer projections
- remove or suspend participants within server policy

The app bridge should reuse existing kernel command and workflow machinery
where possible. It should not become a parallel session authority. The bridge
is an authenticated service caller that asks the kernel to mutate or invoke
runtime state.

Current code already has a hosted-service caller concept, which is a good fit
for app bridge authorization. Current workflow endpoint invocation already has
the right general shape, but Arroba Server needs structured event/action input
and server-app authorization rather than a publication-only route wrapper.

## Programming Model

The first programming model should be a TypeScript SDK over the app bridge,
not a new Arroba Server language.

Shell commands remain useful for manual setup, inspection, admin operations,
and drills. They are not enough for dynamic server apps because app behavior
needs event handlers such as "when a client joins, create a node, create an
endpoint, assign a seat, and subscribe to output."

The SDK should be framework-neutral. It should run in ordinary Node services
and be usable from Express, Fastify, Hono, Next.js route handlers, workers, or
custom daemons. Framework adapters can be added later, but the core SDK should
not depend on Next.js or any web framework.

Conceptual SDK shape:

```ts
server.onClientJoined(async (ctx, client) => {
  const session = await ctx.sessions.ensure({ name: "poker-main" });
  const workflow = await ctx.workflows.ensure({
    sessionId: session.id,
    name: "table-1",
  });

  const node = await ctx.workflows.addAgentNode({
    workflowId: workflow.id,
    agentId: client.agentId,
    instructions: "You are seated at table 1.",
    outputSchema: pokerActionSchema,
  });

  const endpoint = await ctx.workflows.createEndpoint({
    workflowId: workflow.id,
    entryNodeId: node.id,
    alias: `seat-${client.seat}`,
  });

  await table.assignSeat(client.id, endpoint.id);
});
```

The SDK should wrap typed app bridge messages and kernel responses. It should
not shell out as its primary mechanism, though an SDK command runner can exist
for early compatibility and local drills.

## Structured Events And Outputs

The server app should emit structured event/action requests, not free-form
prompts. Natural language remains useful in workflow and node instructions,
where it describes the environment, rules, and role expectations. Runtime input
from the app should be schema-shaped data.

Example poker input:

```json
{
  "kind": "poker.turn",
  "table_id": "table-1",
  "hand_id": "hand-42",
  "seat": 3,
  "legal_actions": [
    { "kind": "fold" },
    { "kind": "call", "amount": 20 },
    { "kind": "raise", "min": 40, "max": 200 }
  ],
  "public_state": {
    "pot": 120,
    "board": ["Ah", "7d", "2c"],
    "positions": ["button", "small_blind", "big_blind"]
  },
  "private_state": {
    "hole_cards": ["Ks", "Kh"]
  }
}
```

Example output:

```json
{
  "kind": "raise",
  "amount": 60,
  "reason": "Top pair blocker pressure with a strong overpair."
}
```

The server kernel validates the structured output against the workflow node or
endpoint schema. The app server then validates whether the output is legal in
the deterministic application state.

## Workflow Topology

Arroba Server apps should not assume one workflow topology.

For poker, each seated agent can be an independent workflow node with its own
endpoint. The deterministic game app invokes only the active seat's endpoint on
that player's turn. The workflow graph may have no edges between player nodes.

Other applications may need:

- one endpoint per participant
- one shared endpoint for a group
- fan-out to many agents
- fan-in through a judge or moderator agent
- chained workflows
- background watchdogs
- observer-only projections

The app bridge and SDK should therefore expose workflow topology as a
programmable kernel resource rather than baking poker's topology into the
server OS.

## Observer Frontends

Humans should be able to observe Arroba Server activity through a frontend.
This frontend is an observation window, not the primary control authority.

For a poker app, the observer frontend can render a table, seats, cards,
actions, stack sizes, and history. Humans should not directly click buttons to
play for the agents unless a specific app intentionally grants that role.

The observer projection should be served by the Arroba Server app side, based
on kernel and app events. It should not require humans to inspect provider
traces to understand what happened.

Long term, an Arroba-specific browser or viewer may make sense, but that is a
later product layer. V1 can use normal web technology and a non-authoritative
observer UI.

## Identity And Attestation

ArrobaNet needs official kernel identity signing. This is separate from full
hardware or operating-system attestation.

For the proof of concept, the realistic target is:

- official Arroba distributions generate or receive kernel identity keys
- kernels sign connection and policy claims
- Arroba Cloud verifies known official kernel identities
- server policies can require signed kernel identity
- stricter servers can require execution inside an official Arroba slice
- official slices can produce a narrower attestation claim for slice policy

This does not prove that every host operating system is uncompromised. It does
give the protocol a credible trust boundary for official kernels and official
slice environments, which is enough for the first ArrobaNet proof of concept.

## Relationship To Existing Runtime

Arroba Server should extend existing runtime concepts rather than creating a
parallel runtime:

- public admission generalizes collaboration from invite-only to public server
  admission
- server-to-client triggering should use workflow endpoints and remote kernel
  routing
- client kernels continue to own provider execution and terminal fanout
- workflow schemas continue to validate structured outputs
- runtime MCP remains the agent-facing way to request Arroba actions
- hot reload should extend existing provider reload policy
- slice constraints should extend existing slice and remote worker placement

The right architecture is shared core crates/packages, not a long-lived
copy-paste fork. Product repositories can still be separate, but protocol,
kernel-core, workflow-core, transport, identity, and policy code should be
shared where behavior is common.

Potential long-term package split:

```text
arroba-core
  protocol
  identity
  address
  transport
  kernel-core
  workflow-core
  extension-policy
  slice-policy
  app-bridge

arroba
  normal user kernel product
  CLI/TUI
  provider adapters
  collaboration UX

arroba-server-os
  providerless server kernel profile
  public admission
  app bridge
  observer projections
  server app host

arroba-cloud
  resolver
  registry
  official kernel identity
  public server records
  hosted waiting room and control plane
```

## Example: Poker Agents

Poker is a strong first Arroba Server application because it separates
deterministic application authority from agent decision-making.

Server-side authority:

- table creation
- seat assignment
- deck shuffle and hidden information
- legal action calculation
- betting rules
- hand settlement
- play-money balances
- observer state

Agent authority:

- choose an action from legal actions
- provide optional reasoning or table talk
- react to new hand and table events

Kernel authority:

- admit clients
- enforce compliance requirements
- bind agents to workflow nodes
- invoke endpoints
- route events to client kernels
- validate structured outputs
- persist workflow and runtime state
- project terminal and observer events

This division avoids making the LLM the game engine and gives the app server a
clean place to enforce rules.

## Open Design Questions

Provider thread transfer into slices remains the main unresolved design
question. The next step is drills that prove whether Codex, OpenCode, and
Claude can resume the same provider thread inside a standard Arroba slice with
credentials and required provider-local state available.

Other important questions:

- exact app bridge wire shape and authorization model
- structured input shape for workflow endpoint invocations
- server admission records and public participant visibility
- slice attestation claim format
- official kernel identity issuance and rotation
- observer projection protocol
- framework-neutral TypeScript SDK package shape
- minimum protocol changes needed to keep the new behavior shared across local
  CLI, remote TUI, web, and future native clients

