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

## Executable runtime parity evidence contract

The static Path-1 source assertions are not acceptance evidence. The executable
matrix is owned by:

`
scripts/managed-path1-provider-parity-drill.mjs
scripts/managed-path1-provider-parity.test.mjs
scripts/managed-path1-provider-parity-drill.test.mjs
`

Capture must be invoked inside the official provider turn or an approved remote
command boundary on each target. It must run on Linux, use the same reviewed
kernel/provider build, declare @@ordinary@@ or @@path1@@, and write one signed
machine-readable snapshot:

`bash
node scripts/managed-path1-provider-parity-drill.mjs capture --topology ordinary --reviewed-commit "$CHARIOX_PARITY_REVIEWED_COMMIT" --build-id "$CHARIOX_PARITY_BUILD_ID" --provider "$CHARIOX_PARITY_PROVIDER" --provider-version "$CHARIOX_PARITY_PROVIDER_VERSION" --boundary official-provider-turn --fresh-worker --inside-provider-turn --output ordinary.json
`

The same command is run inside the fresh Path-1 worker with
`--topology path1`. The signing key is read from
`CHARIOX_PARITY_SIGNING_KEY`; it is never printed or stored in a snapshot.
Capture probes must be supplied by the provider/remote boundary through the
`CHARIOX_PARITY_*` context fields. Missing context produces a missing result,
not a pass.

Comparison is a separate fail-closed operation:

`bash
node scripts/managed-path1-provider-parity-drill.mjs compare --ordinary ordinary.json --path1 path1.json --reviewed-commit "$CHARIOX_PARITY_REVIEWED_COMMIT" --build-id "$CHARIOX_PARITY_BUILD_ID" --report parity-report.json
`

The comparator requires valid signatures, the same reviewed commit and build
identity, the declared @@ordinary@@/@@path1@@ topologies, a fresh worker, an
official provider identity, and a runtime provider-turn/remote-command
boundary. It rejects source-only captures, unsigned or tampered snapshots,
missing rows, topology/head mismatches, different normalized results, and
cleanup failures. It emits row names and statuses only; provider output,
environment contents, credentials, and command output are not printed.

The required result fields are:

`
exact_cwd
arbitrary_accessible_directory
home_access
tmp_access
post_enrollment_repository
git_file_terminal
provider_ancestry
managed_isolation_environment
mount_visibility
privilege_state
network_reachability
package_tool_installation
official_provider_identity
reconnect_history_result_identity
errors
cleanup
shutdown_agents_done
shutdown_idle_15m
shutdown_idle_30m
shutdown_minimum_3h
shutdown_manual
shutdown_custom
shutdown_explicit_lifecycle_reconciliation
shutdown_deployment_reconciliation
`

`provider_ancestry` must prove no Bubblewrap ancestor on Path 1;
`managed_isolation_environment` must prove the managed isolation and Bubblewrap
environment markers are absent. The remaining rows compare exact cwd semantics,
arbitrary accessible directory create/read/write, `/home`, `/tmp`, a repository
created after enrollment, Git/file/terminal behavior, mount visibility,
`NoNewPrivs`, `CapEff`, umask, network reachability, the permitted
package/tool probe, official provider identity, reconnect/history result
identity, structured errors, and cleanup.

The shutdown rows are mandatory even when a policy means that automatic stop is
not expected. They cover all configured modes (`agents_done`, 15-minute idle,
30-minute idle, minimum three-hour runtime, manual, and custom) plus explicit
lifecycle and deployment reconciliation. Each row must report its observed
outcome; omission is a failed comparison.

The focused Node tests contain green-parity, tamper, missing-row,
topology/head-mismatch, different-result, and cleanup-failure fixtures. They
validate the comparator contract only. A successful focused test run does not
replace live capture from an ordinary Linux kernel and a fresh Path-1 worker;
that live execution remains required before parity is accepted.
