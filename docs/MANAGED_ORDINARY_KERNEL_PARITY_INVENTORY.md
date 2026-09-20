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
provider account home and is deliberately not consulted for repository or
empty-workspace placement. Path 1 does not use that home as a Bubblewrap or
managed-only filesystem boundary.

The default repository destination is
`/home/chariox/<validated-source-basename>`. The machine-level trusted
`repository_root` setting may select another absolute root when the managed
machine is created. Cloud provisioning, bootstrap, kernel receipt, child-worker
creation, and Web projections must carry one server-authoritative value. A
browser, worker, or session cannot supply a second override. Any serialized
protocol change must follow the protocol-version and snapshot/hash rules in
`AGENTS.md`.

## Executable runtime parity evidence contract

The static Path-1 source assertions are not acceptance evidence. One executable
contract owns capture, validation, and comparison:

```text
apps/cli/scripts/managed-ordinary-parity-collector.mjs
apps/cli/scripts/managed-ordinary-parity-matrix.mjs
apps/cli/scripts/managed-ordinary-parity-collector.test.mjs
apps/cli/scripts/managed-ordinary-parity-matrix.test.mjs
```

The older `scripts/managed-path1-provider-parity-drill.mjs` schema is not an
acceptance source for this matrix. Do not add another snapshot format. Any
remaining useful runtime probes from that script must move behind the
repository-owned collector contract above.

Capture must be invoked inside the official provider turn or an approved remote
command boundary on each target. It must run on Linux, use the same reviewed
kernel/provider build, declare @@ordinary@@ or @@path1@@, and write one signed
machine-readable manifest. The collector uses a repository-owned probe whose
source/build identity must match the reviewed commit. It does not accept a
caller-selected probe executable.

```bash
pnpm --filter @chariox/cli run managed-ordinary-parity:collect -- capture \
  --topology ordinary \
  --reviewed-commit "$CHARIOX_PARITY_REVIEWED_COMMIT" \
  --build-id "$CHARIOX_PARITY_BUILD_ID" \
  --kernel-protocol "$CHARIOX_PARITY_KERNEL_PROTOCOL" \
  --relay-protocol "$CHARIOX_PARITY_RELAY_PROTOCOL" \
  --provider "$CHARIOX_PARITY_PROVIDER" \
  --provider-command "$CHARIOX_PARITY_PROVIDER_COMMAND" \
  --kernel-binary "$CHARIOX_PARITY_KERNEL_BINARY" \
  --boundary official-provider-turn \
  --output ordinary.json
```

The same command is run inside the fresh Path-1 worker with
`--topology path1`. The signing key is read from
`CHARIOX_PARITY_SIGNING_KEY`; it is never printed or stored in a snapshot.
Capture probes must be supplied by the provider/remote boundary through the
`CHARIOX_PARITY_*` context fields. Missing context produces a missing result,
not a pass.

Comparison is a separate fail-closed operation:

```bash
pnpm --filter @chariox/cli run managed-ordinary-parity:compare -- \
  --ordinary ordinary.json \
  --path1 path1.json \
  --reviewed-commit "$CHARIOX_PARITY_REVIEWED_COMMIT" \
  --build-id "$CHARIOX_PARITY_BUILD_ID" \
  --report parity-report.json
```

The comparator requires valid signatures, the same reviewed commit and build
identity, the declared `ordinary` and `path1` topologies, a fresh worker, an
official provider identity, and a runtime provider-turn/remote-command
boundary. It rejects source-only captures, unsigned or tampered snapshots,
missing rows, topology/head mismatches, different normalized results, and
cleanup failures. It emits row names and statuses only; provider output,
environment contents, credentials, and command output are not printed.

`ROW_DEFINITIONS` in `managed-ordinary-parity-matrix.mjs` is the sole
machine-readable list of required rows and checks. It groups the contract as
`MP-01` through `MP-10`. The plan-level `MP-11` source inventory is a separate
proactive audit gate and cannot be represented by a runtime snapshot alone.

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
