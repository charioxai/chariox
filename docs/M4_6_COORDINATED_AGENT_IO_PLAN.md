# M4.6 Coordinated Agent I/O Plan

M4.6 is the dedicated milestone for kernel-owned artifact I/O coordination. It is intentionally separate from M4.5: M4.5 retires runtime/facade compatibility paths, while M4.6 defines and enforces safe concurrent writes for Arroba-managed agents.

## Goal

Arroba should be unopinionated about branches, worktrees, and whether multiple agents work in the same repo, branch, file, or artifact. The guarantee is narrower and stronger:

- Arroba-managed agents must never write overlapping regions of the same artifact at the same time.
- Non-overlapping concurrent edits in the same artifact must be allowed.
- Whole-file or whole-worktree blocking is not acceptable for artifact domains that support finer conflict regions.
- External user/process changes are allowed, but Arroba-managed provider sessions must not be able to bypass the coordinated write path.
- If a provider cannot be configured or sandboxed so writes go through Arroba, that provider mode is unsupported for the coordinated-I/O guarantee.

This is a synchronization problem, closer to operating-system concurrency control than to Git merge conflict cleanup. Git conflict detection is too late and too coarse to be the primary mechanism.

## Hard Guarantee Boundary

The M4.6 guarantee applies only to writes performed by agents launched and managed by Arroba provider sessions.

For those sessions, Arroba must enforce write permissions so the provider cannot mutate coordinated workspace artifacts except through Arroba's managed I/O tools. Detection-only or best-effort warnings are insufficient for managed agents.

Unsupported cases:

- A provider session that can directly mutate the repo outside Arroba's edit API.
- A shell/tool configuration that gives unrestricted write access to coordinated artifacts.
- A remote worker that claims to be coordinated but cannot route writes through the home kernel.

Allowed but outside the guarantee:

- Human edits in the worktree.
- Independent external processes not launched as Arroba-managed agents.
- Other kernels or agents not joined to the same coordinated workspace.

External edits are still observed and reconciled before managed writes are applied, but they are not prevented by Arroba unless the process is an Arroba-managed provider session.

## Artifact Domains

M4.6 should model conflict semantics by artifact domain, not by file extension alone. A file is just a storage container; the artifact domain defines regions, changed areas, rebase rules, and conflict checks.

Initial domains:

| Domain | Region model | First support |
| --- | --- | --- |
| `TextDocument` | byte ranges, line ranges, hunks with context | Required |
| `StructuredDocument` | document paths, keys, array identities, cells where applicable | Optional first slice if cheap |
| `OpaqueBlob` | whole artifact | Required fallback for unsupported binary formats |

Future domains:

| Domain | Region model examples |
| --- | --- |
| `ImageAsset` | layer IDs, pixel rectangles, masks, metadata fields |
| `VectorAsset` | element IDs, paths, groups, layers |
| `AudioAsset` | track IDs, time ranges, clips, metadata |
| `VideoAsset` | track IDs, time ranges, frames, subtitles, metadata |
| `PdfDocument` | pages, annotations, extracted text regions, embedded assets |
| `DirectoryArtifact` | subtree paths and generated bundle members |

Unsupported artifact domains must not silently become unsafe. They either use conservative whole-artifact conflict semantics or require an explicit unsupported/unsafe mode that does not claim M4.6 guarantees.

## Core API Shape

Agents should submit edit intent, not trusted coordination metadata. The kernel enriches requests with identity, version, workspace, provider, and snapshot data.

Agent-facing intent examples:

```rust
struct AgentEditIntent {
    path: PathBuf,
    snapshot_id: Option<ArtifactSnapshotId>,
    operation: AgentEditOperation,
}

enum AgentEditOperation {
    ReplaceText { old_text: String, new_text: String },
    ApplyPatch { patch: String },
    WriteArtifact { bytes: Vec<u8> },
}
```

Kernel-owned request:

```rust
struct EditRequest {
    agent_id: AgentId,
    session_id: SessionId,
    provider_run_id: ProviderRunId,
    workspace_identity: WorkspaceIdentity,
    artifact_id: ArtifactId,
    domain: ArtifactDomain,
    base_snapshot_id: Option<ArtifactSnapshotId>,
    base_version: Option<ArtifactVersion>,
    operation: DomainEditOperation,
}
```

The kernel, not the agent, owns:

- agent/session/provider identity
- workspace and branch identity
- artifact identity and canonical path
- snapshot and version lookup
- changed-region computation
- conflict checks
- rebase/apply decisions

## Managed Reads

Managed reads are required for best precision and should be the normal provider path. A read returns content plus coordination metadata:

```rust
struct ArtifactReadResult {
    artifact_id: ArtifactId,
    path: PathBuf,
    domain: ArtifactDomain,
    version: ArtifactVersion,
    snapshot_id: ArtifactSnapshotId,
    content: ArtifactContent,
}
```

Read restriction is not the safety boundary. Reads matter because they bind later edits to the version the agent observed. Shell reads may be difficult to forbid in every provider harness, but writes are the hard line.

If an edit lacks a known snapshot, the coordinator may still accept it if the operation contains enough exact old content/context to infer the intended base safely. Otherwise it must reject with a structured request for a managed reread.

## Managed Writes

All coordinated writes must go through the `ArtifactEditCoordinator`.

Apply algorithm:

1. Receive agent edit intent through the managed MCP/runtime tool path.
2. Kernel enriches the intent into an `EditRequest`.
3. Resolve current workspace identity and artifact identity.
4. Verify the provider session is still in coordinated write mode.
5. Serialize apply for the target artifact.
6. Refresh current artifact hash/version before applying.
7. Compute changed regions from base snapshot to current artifact.
8. Compute touched regions for the requested operation.
9. If regions overlap, reject with structured conflict data.
10. If regions do not overlap, rebase operation to current coordinates when needed.
11. Apply the operation synchronously.
12. Persist the new snapshot/version.
13. Return success, or success-with-warning if the edit was rebased over non-overlapping changes.

Result shape:

```rust
enum EditResult {
    Applied { new_version: ArtifactVersion },
    AppliedWithWarning { new_version: ArtifactVersion, warning: EditWarning },
    Rejected { reason: EditRejection },
}
```

Policy:

- If no conflict arises, allow the edit.
- If the artifact changed but the edit rebases cleanly with no overlapping regions, apply and notify the agent with a warning.
- If regions overlap or safety cannot be proven, reject and tell the agent to reread/retry.

## Conflict Domains

Conflict detection should be pluggable:

```rust
trait ConflictDomain {
    type Snapshot;
    type Operation;
    type Region;

    fn snapshot(bytes: &[u8]) -> Self::Snapshot;
    fn touched_regions(base: &Self::Snapshot, operation: &Self::Operation) -> Vec<Self::Region>;
    fn changed_regions(base: &Self::Snapshot, current: &Self::Snapshot) -> Vec<Self::Region>;
    fn conflicts(requested: &[Self::Region], changed: &[Self::Region]) -> bool;
    fn rebase(
        operation: Self::Operation,
        base: &Self::Snapshot,
        current: &Self::Snapshot,
    ) -> RebaseResult<Self::Operation>;
    fn apply(current: &mut Self::Snapshot, operation: Self::Operation) -> ApplyResult;
}
```

`TextDocument` is the first required implementation. It should support exact text replacements and patch hunks, compute changed/touched ranges, and rebase non-overlapping stale edits.

## Provider Permission Enforcement

M4.6 must include provider-specific investigation and implementation for Codex and OpenCode, with Claude Code following the same model once the provider lands.

Required outcomes for each supported provider:

- Provider file writes are routed through Arroba managed MCP/runtime tools.
- Direct writes to coordinated workspace artifacts are blocked by provider configuration, sandboxing, or filesystem permissions.
- Shell/tool write access is either disabled for coordinated paths or confined to explicitly allowed scratch/output directories.
- If the provider cannot be configured this way, Arroba must mark that provider/session mode unsupported for coordinated I/O.

Candidate enforcement mechanisms, to be evaluated per provider:

- Provider tool-surface configuration that removes native write tools and exposes only Arroba MCP write tools.
- Read-only workspace mount plus writable scratch/cache directories.
- Provider sandbox or container/jail with write deny rules for coordinated artifacts.
- Overlay workspace where the provider cannot commit writes directly to the canonical worktree.
- FUSE or virtual filesystem interception as a later stronger option if provider configuration is insufficient.

Detection is still useful for diagnostics, but it is not a substitute for blocking direct writes by Arroba-managed agents.

## External Changes

External changes from humans or non-Arroba processes are not blocked. The coordinator observes them before managed writes:

- managed reads create active artifact snapshots
- file watchers mark active artifacts dirty
- pre-apply hash verification is authoritative
- stale snapshots are diffed against current content
- overlapping managed edits are rejected
- non-overlapping managed edits are rebased and applied

Agent notification rules:

- External/non-overlapping change: apply with warning.
- External/overlapping change: reject with conflict regions and reread instruction.
- External change without enough base context: reject with reread instruction.

## Workspace Identity And Remote Agents

Remote coordination is required only when agents are working in the same logical repo and branch/workspace as local agents. Different repos, branches, directories, or unjoined workspaces are outside live I/O coordination.

Workspace identity must not be treated as static. Local and remote workspace identity can change at any time through checkout, branch switch, remote rewrite, worktree movement, or repo reconfiguration. The kernel must detect identity changes and reclassify whether a provider run belongs to a coordinated workspace.

Identity inputs:

```rust
struct WorkspaceIdentity {
    vcs_provider: Option<VcsProvider>,
    repo_id: Option<String>,
    repo_url: Option<String>,
    branch: Option<String>,
    head_commit: Option<String>,
    worktree_root_fingerprint: String,
}
```

For GitHub-backed workspaces, prefer GitHub repository ID when available, with normalized remote URL as fallback. GitHub repo identity is an input, not the only authority.

Coordination requires:

- same logical repository identity
- same branch identity where branch semantics apply
- compatible workspace root/worktree fingerprint
- explicit membership in the same Arroba coordinated workspace group
- current identity check before coordinated read/write operations

If identity changes and no longer matches, the kernel must stop routing that provider run through the shared coordinator and notify the agent. If identity changes into a matching coordinated workspace, the kernel may join it after validation and explicit association.

Remote edit flow for coordinated workspaces:

1. Remote worker reports current workspace identity to the home kernel.
2. Home kernel validates compatibility and coordinated membership.
3. Remote reads/writes route through the home artifact coordinator.
4. Home coordinator applies or rejects writes.
5. Remote workspace receives accepted patches/sync updates.

The relay remains transport. The home kernel remains the workspace and artifact-I/O authority.

## MCP/Runtime Tool Surface

M4.6 should expose managed I/O through the existing runtime tool/MCP path.

Initial tools:

- `arroba.read_artifact`
- `arroba.edit_artifact`
- `arroba.apply_patch`
- `arroba.write_artifact` internally converted to a domain operation/diff
- `arroba.inspect_artifact`

Tool responses must include structured success, warning, and rejection payloads so agents can reread and retry deterministically.

## Milestone Slices

1. Define artifact identity, artifact versions, snapshots, and conflict-domain traits.
2. Add managed artifact read tool returning snapshot/version metadata.
3. Add managed text edit/apply-patch tool with synchronous apply/reject semantics.
4. Add per-artifact serialized `ArtifactEditCoordinator`.
5. Implement `TextDocument` changed/touched region detection.
6. Implement non-overlapping stale edit rebase for text.
7. Add filesystem watcher plus authoritative pre-apply hash verification for active artifacts.
8. Add structured conflict/warning responses and agent-facing retry instructions.
9. Investigate and implement Codex provider write-permission enforcement.
10. Investigate and implement OpenCode provider write-permission enforcement.
11. Wire provider sessions so coordinated workspace writes can only use managed Arroba tools.
12. Add workspace identity snapshots and identity-change detection for local provider runs.
13. Add remote workspace identity handshake design and protocol docs.
14. Add remote coordinated edit routing through the home kernel for matching workspaces.
15. Add unsupported-provider/session-mode reporting when write enforcement cannot be guaranteed.
16. Extend to `StructuredDocument` domains where safe.
17. Design later image/audio/video/PDF/vector artifact domains.

## Non-Goals

- Whole-file locking for text/code artifacts.
- Whole-worktree locking as the primary conflict mechanism.
- Relying on Git conflict detection as the primary safety mechanism.
- Best-effort direct-write detection as a substitute for provider write blocking.
- Coordinating unrelated directories, unrelated branches, or unjoined workspaces.
- Making relay the workspace or artifact authority.
