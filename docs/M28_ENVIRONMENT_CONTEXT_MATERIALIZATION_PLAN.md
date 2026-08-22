# M28: Environment context materialization

Status: implemented on `codex/managed-remote-kernels`; hosted acceptance pending

Scope: `chariox` runtime, managed slices, and independently authoritative managed kernels

Cloud companion: `chariox-cloud/docs/C9_MANAGED_REMOTE_KERNELS_MILESTONE.md`

## 1. Runtime contract

Chariox materializes development context for two distinct topologies:

- A managed slice remains owned by its parent kernel. The parent owns the session,
  agents, prompt history, grants, and runtime interactions. The slice is only the
  selected execution environment.
- A managed remote kernel is independent. It owns its own identity, sessions,
  agents, provider runs, history, workspaces, child slices, and terminal streams.
  It is never represented as a home-worker lease.

Both paths reuse the same Project repository selection and development snapshot
logic. They do not share runtime authority.

## 2. Context layers

The implemented launch plan keeps four layers separate:

1. The immutable managed-kernel release and bootstrap supervisor.
2. Kernel context, either Empty or copied from one connected source kernel.
3. Development context, either Empty or an exact Project repository selection.
4. Provider and SCM runtime material selected from that same source kernel.

The user-facing source field is `Kernel context from`. Empty installs the managed
release with default extensions and an empty Vault. A connected source copies the
portable kernel context and complete encrypted Vault snapshot. Development and
provider material can still require a source when kernel context is Empty. All
source-backed layers must name the same connected source target.

Sessions, agents, provider runs, prompts, grants, history, attachments, runtime
interactions, and source machine identity are never transferred.

## 3. Direct transfer

Cloud authorizes and binds a transfer. It stores the normalized launch plan,
including source target, Project, Workspace, worktree, provider-account, and SCM
handle metadata, plus hashes, coarse phase, and redacted errors. Source and target
kernels exchange the package directly over the encrypted relay peer channel. The
relay sees encrypted routing traffic. Neither Cloud nor the relay receives package
plaintext or credential values.

The source kernel:

- fetches an authoritative, short-lived transfer ticket from Cloud;
- exports the selected kernel, development, provider, and SCM components;
- persists the prepared package and operation status under its private Chariox
  state root;
- sends bounded chunks to the exact target kernel;
- resumes from the target's committed offset after disconnect or restart; and
- treats a matching completed receipt as authoritative after a lost finalize
  response.

The target kernel:

- validates the source and target relay identities, plan digest, package digest,
  expiry, capacity, and single-consumer binding;
- commits chunks and transfer state durably before acknowledging them;
- imports components into isolated staging paths;
- publishes the final launch target only after every selected component succeeds;
  and
- preserves consumed receipts and launch targets across restart for idempotent
  replay.

A retryable source or transport failure remains visible to the initiating client.
The launch never falls back to Empty.

## 4. Kernel context

The kernel package contains the portable kernel configuration, unified Extension
registry, extension packages, scopes, and a target-sealed Vault snapshot. Supported
extension kinds include MCP, Skill, Script, and Connector. Package hashes and
configuration remain bound to the transfer manifest.

The target creates its own kernel identity before import. It does not import the
source identity or source runtime database.

## 5. Development context

A Project can name several Workspaces. At launch the user selects one primary
Workspace and an explicit repository set. The primary repository is mandatory.
Supporting repositories default from the Project and can be removed or restored
before launch.

Each selected repository is exported independently with its Git branch, commit,
tracked files, untracked files, and working state. The target materializes every
repository under its own target Workspace path and reports those target paths in
the launch receipt. The imported Project is registered in the target kernel before
the client creates the session, so Project selection and slice validation use the
target's authoritative Workspace records.

Managed slices accept the same repository topology. Slice reuse requires an exact
topology match and never changes the parent-kernel ownership model.

Repository `AGENTS.md` and `CLAUDE.md` files remain normal repository content. A
separate development environment-variable and secret layer is not implemented in
this milestone. Values the user deliberately stores in the kernel Vault are part
of the complete Vault snapshot when source kernel context is selected. Empty
kernel context does not copy that Vault.

## 6. Provider and SCM runtime

Selected Codex, OpenCode, and Claude account profiles are exported and restored
through their official provider-local file and CLI mechanisms. Chariox does not
use provider SDKs or alternate hosted agent services. Claude headless and native
profiles resolve to the same canonical Claude account material.

The first SCM implementation supports an explicitly selected GitHub credential.
SCM import uses the existing safe slice baseline and configures target-local Git
credential behavior without sending credential values through Cloud or relay
control records.

## 7. Empty context

Empty context creates a durable target Workspace owned by the independent kernel.
The launch receipt contains that target Workspace and worktree path. This path is
not a source-machine placeholder and remains valid across client retry and kernel
restart.

## 8. Client behavior

The TUI and web Waiting Room use the ordinary Machine field. Selecting
`+ New Chariox-managed machine...` reveals compute class, region, kernel-context
source, development setup, repository selection, provider accounts, Git
credentials, and auto-stop policy. Ordinary machines retain their existing rows.

The client persists one launch attempt identity through retries, waits for Cloud
provisioning and relay readiness, performs any direct source transfer, switches to
the target kernel, verifies the launch target, and only then creates the session.
Cancellation never silently creates a local session.

## 9. Detached execution and activity

The independent kernel remains the authority when every terminal disconnects.
Active provider turns continue, queued prompts advance, outputs and tool results
are written to durable agent-scoped history, and unresolved interactions remain
available for later attachments. Zero-recipient live output is not retained in the
bounded terminal fanout queue because durable history is the recovery source.

The managed activity reporter maintains a signed monotonic cursor over a binary
zero or nonzero running-agent count. It sends initial state, count transitions,
restart resynchronization, lost-ack replay, and corrective same-count cursor
updates. Active prompts, queued prompts, provider settlement, and unresolved
interactions count as work. The kernel persists response and prompt settlement
state before it reports zero.

## 10. Protocol and validation

The local daemon protocol is version 278 for this implementation. Shape tests cover
managed environment summaries and control, transfer preparation and status,
explicit launch-target requests, multi-Workspace Projects, slice repository
topology, and provider and SCM selection. Managed activity uses a separately
authenticated Cloud HTTP contract with canonical signature vectors and cursor
tests.

Focused tests cover transfer resume and replay, restart recovery, bounded storage,
identity binding, Vault sealing, exact Project materialization, detached queue
advancement, and zero-recipient history behavior. Final acceptance still requires
the hosted relay and provider matrix listed in the goal and Cloud milestone.
