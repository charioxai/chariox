# Managed/ordinary kernel parity inventory

This inventory records the user-visible managed/ordinary kernel seams audited for
the PR #363 workspace-materialization change. Path 1 uses the disposable VM as
its provider boundary, so provider processes use ordinary worker permissions.
Root-owned control data, signed releases, credential hygiene, resource policy,
and managed lifecycle controls remain in force without changing provider
filesystem semantics.

| Area | Ordinary kernel | Managed kernel | Disposition |
| --- | --- | --- | --- |
| Directory search, list, and create | Searches the current directory and `HOME`, lists readable directory children, and creates the requested directory. | Uses the same local search/create contract, with managed launch targets available as additional selected paths. | Exact directory queries now remain valid when child enumeration is denied; child `PermissionDenied` is best-effort and does not erase the exact directory. |
| Arbitrary cwd selection | Uses the selected workspace/worktree path directly. | Path-1 providers use the same selected cwd under ordinary worker permissions, without Bubblewrap or a managed-only path allowlist. | Docker-slice or explicitly documented shared-host inner boundaries are separate topologies and do not define Path-1 behavior. |
| Empty workspace creation | Uses the ordinary configured state/workspace layout. | Uses a user workspace under `/home/chariox/.chariox-empty-context-<context-hash>` when managed control state is configured; the completion receipt remains in managed durable state. | User workspace and control/receipt state are separate. |
| Copied repository destination | Ordinary imports retain their caller-provided destination root. | Managed published repositories use `/home/chariox/<validated-source-basename>`. Control publication directories and receipts remain below the managed private state root. | The fixed default is used because no dedicated trusted repository-root setting exists. Existing destinations are rejected; no overwrite or hash/suffix fallback is used. |
| Repository basename | Existing ordinary export naming remains source/worktree-driven. | The source worktree basename is preserved after single-component safety validation. | Case-insensitive basename collisions are rejected during export and destination collisions are rejected during import. |
| Worktree placement | Ordinary worktrees stay in the existing local worktree layout. | A managed context is a materialized copy; its user-visible repository paths are the `/home/chariox` paths above, while transfer receipts/staging remain private. | Provenance, bundle verification, overlay replay, and no-clobber publication are retained. |
| Session and agent launch | Launch targets point at ordinary local workspace paths. | Managed transfer launch targets are recovered from the confirmed publication and point at materialized repository paths; empty targets use the managed user-workspace path. | Control roots are not selected as repository workspaces. |
| Terminal, file, Git, and provider behavior | Operates on the selected ordinary workspace under ordinary local permissions. | Operates on the selected materialized workspace under the same worker permissions while scrubbing managed control and relay secrets. | The VM, Unix ownership, dedicated service roots, and environment hygiene protect managed infrastructure; Path 1 does not fork provider behavior. |
| Reconnect and orphan recovery | Uses ordinary local session/state recovery. | Uses transfer leases, publication receipts, staging recovery, and no-clobber cleanup/recovery. | Receipt validation remains authoritative; ambiguous paths fail closed. |
| Managed idle shutdown | No managed idle shutdown policy. | Managed lifecycle policy retains every configured trigger: minimum runtime, explicit lifecycle/deployment reconciliation, and idle shutdown whose delay is measured from the last agent finishing. | Managed automatic shutdown is mandatory and intentional; it is not an ordinary-parity bug. |
| Deployment | Local deployment is not part of the ordinary workspace contract. | Managed deployment/bootstrap/reconciliation may differ. | Deployment is the other intentional managed-only difference. |

## State and configuration separation

`CHARIOX_PUBLICATION_CONTROL_STATE_DIR` identifies durable publication/control
state; it is not a repository root. `CHARIOX_MANAGED_PROVIDER_HOME` is the
provider account/isolation home and is deliberately not consulted for repository
or empty-workspace placement. No dedicated trusted machine-level repository-root
setting was found.

The current contract therefore implements only the fixed default
`/home/chariox/<validated-source-basename>`. An optional custom root would need a
new, distinct trusted `repository_root` configuration field, bootstrapped through
a dedicated setting and threaded into managed transfer/import. If that field were
serialized into a public request/config shape, the local daemon protocol version,
snapshots/hashes, and the minimum dependent client version would need the standard
protocol-change update. This slice adds no such field and makes no serialized
protocol change.
