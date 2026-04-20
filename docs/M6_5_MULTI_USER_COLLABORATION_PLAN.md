# M6.5 Multi-User Collaboration Plan

## Goal

Allow two or more users to collaborate in one Arroba session while keeping provider ownership private and keeping the session, workflow, relay, and managed-I/O model coherent.

The v1 collaboration model is open collaboration inside an invited session:

- users share the same session
- users may work in the same repository, branch, and worktree when they choose to
- users may also work across separate forks or worktrees and explicitly link them for Arroba managed-I/O coordination
- each user can see and control only their own providers and agents outside workflow execution
- shared workflow graphs update for all session members
- workflow nodes can represent agents owned by different users
- users can connect their own nodes to other users' nodes without requiring inbound approval
- private agent configuration remains private to the owning user

This milestone does not add a public agent marketplace, external-agent marker, or generalized ACL language. V1 uses fixed ownership rules and session-scoped invites.

## Core Decisions

- A collaboration unit is still one Arroba session.
- Invites are per session.
- The kernel remains the authority for membership, ownership, redaction, workflow mutation, endpoint execution, and managed-I/O coordination.
- The relay remains a transport layer and must not become a policy authority.
- There is no external-agent marker in workflow UI. Public node labels are the shared identifier.
- Freeform mode does not expose other users' agent status.
- Workflow mode may expose individual node/run status because workflow execution depends on whether a node is running, blocked, failed, or waiting.
- Only endpoint owners can run their workflow endpoints.
- Users can create endpoints only for their own nodes.
- Several endpoints can exist in the same workflow, owned by different users.
- Node-level prompts and endpoint prompts are private because they are attached to an owned agent.
- Workflow graph structure, public labels, edges, endpoint aliases, run state, and workflow outputs are visible to session members unless a later milestone adds stronger privacy modes.
- Concurrent workflow edits use simple optimistic rejection. Arroba does not merge simultaneous graph edits in v1.
- Managed I/O remains the file/worktree conflict authority. This milestone does not add another I/O conflict resolver.

## Non-Goals

- external agent marketplace
- public discovery of arbitrary users or agents
- organization-wide permissions
- read-only or restricted collaboration roles
- fine-grained ACLs beyond fixed ownership checks
- secret sharing between users
- remote installation of another user's providers, MCPs, skills, or credentials
- automatic Git relationship inference between forks
- Git merge conflict resolution
- workflow edit merging
- hiding shared workflow outputs after they are emitted into the workflow

## Identity And Membership

Add explicit session membership.

Required concepts:

```rust
struct SessionMember {
    session_id: SessionId,
    user_id: UserId,
    joined_at_ms: u64,
    invited_by_user_id: Option<UserId>,
}

struct SessionInvite {
    invite_id: SessionInviteId,
    session_id: SessionId,
    created_by_user_id: UserId,
    expires_at_ms: Option<u64>,
    max_uses: Option<u32>,
    used_count: u32,
    revoked_at_ms: Option<u64>,
}
```

V1 role model:

- every invited user is a session member
- all members can see shared session/workflow content after joining
- all members can manipulate workflow structure according to node/endpoint ownership rules
- no viewer/editor/admin role split in this milestone

Default migration behavior:

- existing single-user sessions get one implicit local user
- existing agents, workflow nodes, endpoints, and provider configuration become owned by that local user
- local-only use continues to work without visible user-management setup

## Provider And Agent Ownership

Agents and providers are owned by a user.

Rules:

- a user can list, launch, stop, configure, and prompt only their own freeform agents
- a user cannot see another user's provider, model, credential state, runtime profile, endpoint prompt, or node-level prompt
- a user cannot attach another user's provider endpoint to a node
- a user can see another user's workflow node only through its public node label and shared workflow state

Freeform visibility:

- own agents: full status/control
- other users' agents: hidden from freeform agent lists and freeform status surfaces

Workflow visibility:

- shared node status is visible during workflow inspection and workflow runs
- provider/model/private configuration remains redacted
- status must be scoped to the workflow node/run, not exposed as a general freeform agent presence surface

Rationale:

Workflow dispatch uses the same per-agent prompt/runtime and workspace-claim machinery as ordinary prompts. If an agent is busy outside the workflow or blocked on a claim, the workflow cannot treat that node as immediately runnable. Showing workflow node state is therefore part of explaining shared workflow behavior, while freeform status remains private.

## Workflow Ownership Model

### Nodes

Add ownership and public labeling:

```rust
struct WorkflowNodeDefinition {
    node_id: WorkflowNodeId,
    agent_id: AgentId,
    owner_user_id: UserId,
    public_label: String,
    // private to owner:
    instructions: Option<String>,
}
```

Rules:

- a user may add only their own agents as workflow nodes
- a user may edit only their own node's private prompt/instructions
- other members see the node id, public label, and shared graph/run state
- provider/model/runtime profile details are visible only to the owner
- public labels should be enough to identify the node in shared workflow views

### Edges

Edges are shared graph structure:

```rust
struct WorkflowEdgeDefinition {
    from_node_id: WorkflowNodeId,
    to_node_id: WorkflowNodeId,
    created_by_user_id: UserId,
}
```

Rules:

- a user may add an edge when at least one endpoint node is theirs
- a user may remove any edge incident to one of their own nodes
- no inbound approval is required
- edge creator does not get special removal rights beyond the incident-node rule in v1

### Endpoints

Endpoints are owned execution surfaces:

```rust
struct WorkflowEndpointDefinition {
    endpoint_id: WorkflowEndpointId,
    owner_user_id: UserId,
    alias: Option<String>,
    entry_node_id: WorkflowNodeId,
    // private to owner:
    prompt: Option<String>,
}
```

Rules:

- a user may create endpoints only for their own nodes
- a user may bind/rebind an endpoint only to their own nodes
- only the endpoint owner can run the endpoint
- endpoint prompt/configuration is visible only to the owner
- endpoint alias/name and public run state may be visible to session members

## Workflow Edit Concurrency

Add a workflow revision.

Every workflow-mutating command should carry the client's expected revision:

```rust
struct WorkflowMutationEnvelope<T> {
    workflow_id: WorkflowDefinitionId,
    expected_revision: WorkflowRevision,
    mutation: T,
}
```

Behavior:

1. Kernel compares `expected_revision` with the current workflow revision.
2. If it matches, apply the mutation and increment the revision.
3. If it does not match, reject the mutation with a structured stale-workflow error.
4. Client refreshes and asks the user or caller to retry.

V1 rejection shape:

```rust
enum WorkflowMutationError {
    StaleWorkflowRevision {
        expected: WorkflowRevision,
        actual: WorkflowRevision,
    },
    Unauthorized {
        reason: String,
    },
    Conflict {
        reason: String,
    },
}
```

Policy:

- no graph merge
- no last-writer-wins
- no silent rewrite
- shell and TUI should both send expected revisions for interactive workflow edits
- stale mutation errors should be concise and user-facing

## Workspace Links

Workspace links let session members tell Arroba that separate local repositories, forks, branches, or worktrees should be treated as one logical coordination target for managed I/O.

This is not Git synchronization. It is an Arroba coordination identity.

Example use case:

- user A works in `github.com/a/project`
- user B works in `github.com/b/project-fork`
- both are logically collaborating on the same project branch
- they attach their current worktrees to the same Arroba workspace link
- managed I/O coordinates writes as if both worktrees belong to one logical workspace

State shape:

```rust
struct WorkspaceLink {
    link_id: WorkspaceLinkId,
    session_id: SessionId,
    name: String,
    created_by_user_id: UserId,
    created_at_ms: u64,
}

struct WorkspaceLinkAttachment {
    link_id: WorkspaceLinkId,
    user_id: UserId,
    machine_id: MachineId,
    kernel_id: KernelId,
    repo_root: PathBuf,
    branch: Option<String>,
    repo_fingerprint: Option<String>,
    attached_at_ms: u64,
}
```

Rules:

- workspace links are session-scoped in v1
- only session members can list or attach to a session workspace link
- no separate workspace-link invite is needed; the session invite is the access boundary
- attaching a worktree does not change Git remotes, branches, or files
- external changes remain allowed; managed I/O already accounts for observed file state and conflicts
- if a worktree is not attached to a link, Arroba treats it according to the existing workspace identity rules

Command shape:

```bash
arroba workspace link create <name>
arroba workspace link list
arroba workspace link show <name-or-id>
arroba workspace link attach <name-or-id>
arroba workspace link detach <name-or-id>
```

The same command family must be available in `arroba-shell`.

Managed-I/O integration:

- resolve the current physical workspace identity
- check whether it is attached to a session workspace link
- when attached, use the link id as the logical coordination workspace id
- do not add a second conflict policy above managed I/O

## Relay And Privacy Boundary

Relay behavior:

- routes encrypted session traffic between users, kernels, and machines
- forwards membership/join requests to the authoritative kernel path
- does not authorize workflow edits
- does not inspect provider prompts, workflow payloads, or user-generated content

Kernel behavior:

- validates session membership
- validates ownership for every user-scoped action
- redacts projections per caller
- emits shared workflow/session events to all members according to visibility rules

Remote payload rule from M5 still applies:

- all user-generated remote payloads remain session-scoped end-to-end encrypted
- self-hosted relay deployments do not relax that rule

## Protocol And Projection Changes

Protocol requests that mutate session, agent, provider, workflow, endpoint, workspace-link, or invite state must carry caller identity.

Read projections need caller-aware redaction:

- provider list: own providers only
- freeform agent list/status: own agents only
- workflow graph: all shared nodes/edges/endpoints, with private fields redacted
- workflow run view: shared run/node statuses and outputs
- endpoint config: full for owner, redacted for non-owner
- session member list: visible enough to explain collaboration, without provider metadata

Avoid returning private fields and relying on the CLI to hide them. Redaction belongs at the kernel/protocol boundary.

## CLI And Shell Surface

Required TUI or slash commands:

```text
/session invite create
/session invite revoke <invite-ref>
/session members
/workflow node add <agent-ref> --label <label>
/workflow node label <node-ref> <label>
/workflow endpoint new <workflow-ref> <node-ref> [alias]
/workflow endpoint bind <endpoint-ref> <node-ref>
/workflow run <endpoint-ref>
/workspace link create <name>
/workspace link list
/workspace link show <name-or-id>
/workspace link attach <name-or-id>
/workspace link detach <name-or-id>
```

Required shell coverage:

```text
session invite create
session invite revoke <invite-ref>
session members
workflow node add <agent-ref> --label <label>
workflow node label <node-ref> <label>
workflow endpoint new <workflow-ref> <node-ref> [alias]
workflow endpoint bind <endpoint-ref> <node-ref>
workflow run <endpoint-ref>
workspace link create <name>
workspace link list
workspace link show <name-or-id>
workspace link attach <name-or-id>
workspace link detach <name-or-id>
```

The shell executor must share the same kernel authorization path as the TUI. It must not duplicate ownership decisions locally.

## Delivery Slices

### Implementation Status

As of 2026-04-20:

- Slice 1 is closed: sessions now persist an implicit local owner, member records, and session invite records with migration defaults for existing sessions. Kernel API and shell command coverage exists for `session members`, `session invite create`, `session join`, and `session revoke-invite`. Scoped relay claims can carry `user_id` into `KernelCommand.caller.user_id`, and the kernel router now performs a central session-membership preflight for session-scoped requests before state changes. `ListSessions` is filtered to the caller's memberships.
- Slice 2 is closed: agents, workflow nodes, workflow endpoints, workflow edges, launch provider requests, and runtime provider runs now carry durable ownership/creator metadata with defaults for restored single-user data. Spawned agents and workflow nodes/endpoints/edges are assigned from the caller user at the session/workflow runtime boundary. Workflow nodes also carry a public label independent of provider/model configuration.
- Remaining authorization, privacy, conflict handling, workspace links, and relay drills belong to Slices 3-7.
- Slices 3-7 have not started.

### Slice 1. Session Identity And Membership

Add:

- user identity in local kernel context
- session membership records
- session invite creation, revocation, and join flow
- default local-user migration for existing sessions
- caller identity propagation through local and relay request paths

Exit criteria:

- two users can join one session through a session invite
- kernel rejects requests from non-members
- single-user local sessions continue to behave as before

### Slice 2. Ownership Schema

Add owner fields to:

- agents
- workflow nodes
- workflow endpoints
- provider endpoint/config records where needed
- edge creator metadata

Add public node labels.

Exit criteria:

- existing sessions migrate to one owner
- new agents/nodes/endpoints are owned by the caller
- public node labels exist independently of private provider/model configuration

### Slice 3. Kernel Authorization

Status: closed in implementation. The kernel now enforces owner-only control for
freeform agent focus/destroy/capability grants, prompt submit/complete/cancel,
provider launch, remote-agent move, workflow node mutation, endpoint ownership,
endpoint invocation, and collaborative edge add/remove rules.

Enforce:

- users control only their own freeform agents and providers
- users add only their own agents as workflow nodes
- users edit only their own node private prompts/instructions
- users create/bind endpoints only to their own nodes
- only endpoint owners run endpoints
- users add edges only when at least one endpoint node is theirs
- users remove edges only when incident to one of their nodes

Exit criteria:

- unauthorized mutations fail before state changes
- errors are structured and user-facing through `OwnershipAccessDenied`
- regression coverage pins cross-user agent control, prompt submit, workflow
  node mutation, endpoint invocation, and collaborative edge authorization

### Slice 4. Caller-Scoped Projections And Redaction

Status: closed in implementation. Kernel responses are redacted at the
protocol boundary while internal projections remain full-fidelity. Session and
agent listings expose only the caller's freeform agents; workflow graph and run
status stay visible; non-owner workflow node instructions, workflow invocation
prompts, transient turn inputs, provider runs, and provider processes are hidden
or rejected.

Implement projection filtering at the kernel/protocol boundary.

Exit criteria:

- freeform agent status for other users is hidden
- provider/model/private prompts are hidden from non-owners
- workflow graph and workflow node/run status remain visible
- workflow outputs remain shared session content
- TUI and shell receive already-redacted data

### Slice 5. Workflow Revision Conflicts

Status: closed in implementation. Workflow definitions now carry a monotonically
increasing `revision`. Graph, endpoint, and workflow-definition mutations accept
`expected_workflow_revision`; stale values fail with `WorkflowRevisionConflict`
before mutating state, including the current revision for refresh/retry.

Add optimistic workflow revision checks to graph and endpoint mutations.

Exit criteria:

- stale concurrent workflow edits are rejected
- client refresh/retry path is clear
- no silent graph merge or last-writer-wins behavior exists

### Slice 6. Workspace Links

Status: closed in implementation. Sessions now carry session-scoped workspace
links with per-user/per-machine attachments. Members can create, list, show,
attach, and detach links through kernel API, TUI slash commands, and
`arroba-shell`. Managed I/O preserves existing unlinked behavior, but when a
provider run's worktree is attached to a workspace link, coordination uses
`workspace_link:<link-id>` as the logical repository id so explicitly linked
worktrees/forks share edit reservations and artifact snapshots.

Add session-scoped workspace links and shell/TUI commands.

Exit criteria:

- session members can attach separate worktrees/forks to one logical workspace link
- managed I/O uses the link id as the logical coordination workspace id
- existing unlinked managed-I/O behavior remains unchanged
- shell command coverage matches TUI/slash command coverage

### Slice 7. Relay Collaboration Drills

Status: closed for local relay-caller and live relay identity paths. Regression
drills cover remote caller membership, caller-scoped session listing, remote
ownership authorization, projection redaction, remote managed-I/O identity
matching, stale workflow revision rejection, and remote-machine prompt paths.
The live relay identity security drill also passes. Physical remote provider
CLI drills remain an operational follow-up because they require a second
machine/provider environment, not additional M6.5 implementation.

Run live collaboration drills through local and relay-backed paths.

Required drills:

- two users join one session through a session invite
- each user sees only their own freeform agents/providers
- user A adds a workflow node with a public label
- user B sees that node label and adds an edge involving one of B's own nodes
- user B cannot edit user A's node prompt or provider configuration
- user B cannot run user A's endpoint
- user A can run user A's endpoint
- workflow node status is visible during a shared run
- freeform status of other users' agents remains hidden
- stale concurrent workflow edit rejects one mutation with a clear refresh/retry message
- two linked worktrees coordinate managed I/O through one workspace link
- relay path preserves the same authorization and redaction behavior

## Exit Criteria

M6.5 is complete when:

- session-scoped invites allow multi-user session membership
- provider and freeform agent control is owner-only
- shared workflow graphs support nodes owned by multiple users
- public node labels are sufficient to collaborate without exposing provider/model details
- endpoint ownership prevents other users from running or binding private endpoints
- workflow node/run status is visible in workflow context
- other users' freeform agent status remains hidden
- node-level prompts and endpoint prompts are redacted from non-owners
- concurrent workflow edits reject stale mutations cleanly
- workspace links let users coordinate managed I/O across explicitly linked repos/worktrees/forks
- relay-backed collaboration behaves the same as local collaboration
- shell commands exist for every new collaboration command family

## Current Status

As of 2026-04-20, the collaboration foundation has live-drill coverage for the core local/scoped-relay paths:

- `pnpm --filter @arroba/cli run multi-user-workflow:drill` passes against an isolated scoped relay and kernel with three relay callers. It verifies session invites, caller-owned session creation, per-user agent visibility, node ownership, cross-owner edge authorization, unrelated edge-removal denial, stale revision rejection, endpoint-owner invocation denial, incident-edge removal, and non-owner node-instruction redaction.
- `pnpm --filter @arroba/cli run multi-user-cli-workflow:drill` passes with two real PTY-hosted CLIs over scoped relay. It verifies shared-session attach, hidden-agent startup safety, user-owned node creation from each CLI, graph add/remove live refresh in both workflow screens, endpoint creation live refresh, endpoint-owner-only invocation, owner invocation, and workflow-run visibility in both CLIs.
- `pnpm --filter @arroba/cli run multi-user-freeform-relay:drill` passes against an isolated scoped relay and kernel. It verifies that freeform projections remain caller-scoped: each user sees only their own agents, owned-agent prompt submission succeeds, cross-user prompt submission is rejected, and other-user agents are redacted from session state.

The remaining M6.5 validation gap is physical remote-machine/provider-CLI coverage through relay. That is intentionally deferred until the operator can run from a suitable non-remote environment. The cloud-service work does not change the kernel ownership boundary: hosted relay should issue scoped credentials and route packets, while session membership, workflow authorization, provider ownership, and projection redaction remain kernel-owned.

## Open Questions

- Should session member display names be user-chosen aliases, machine-derived labels, or both?
- Should endpoint aliases owned by another user be visible only as public run targets, or should non-owners see a fully redacted endpoint card?
- Should public workflow outputs include all node transcripts or only declared workflow outputs?
- Should there be a later private-output workflow mode, or should v1 keep all workflow outputs public inside the session?
- Should workspace links persist beyond the session after M6.5, or remain session-scoped until there is a project/account model?
