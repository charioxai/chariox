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
apps/cli/scripts/managed-ordinary-parity-probe.mjs
apps/cli/scripts/managed-ordinary-parity-collector.test.mjs
apps/cli/scripts/managed-ordinary-parity-matrix.test.mjs
apps/cli/scripts/managed-ordinary-parity-probe.test.mjs
```

The retired `scripts/managed-path1-provider-parity-drill.mjs` implementation
and its v1 snapshot schema have been removed. The collector and matrix above
are the only runtime parity evidence contract. Do not restore, wrap, or add a
second snapshot format; add any required observation behind the repository-owned
collector and its v2 matrix schema.

Capture must be invoked inside the official provider turn or an approved remote
command boundary on each target. It must run on Linux, use the same reviewed
kernel/provider build, declare `ordinary` or `path1`, and write one signed
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

For MP-10 product-route capture, set `CHARIOX_KERNEL_URL` to the selected
kernel's normal loopback WebSocket endpoint or its authenticated TLS relay
endpoint. A relay capture uses the existing
`CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN` context and an exact kernel locator in
`CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON`; credentials must stay in the approved
provider context and must never appear in endpoint URLs, logs, or manifests.
The locator does not establish capture authority. The collector obtains the
provider run, session, prompt, attachments, and kernel identity from the normal
product client, then independently checks the provider-child ancestry and the
running signed kernel executable through Linux process metadata.

MP-10 admission and evidence signing are separate. The HMAC in
`CHARIOX_PARITY_SIGNING_KEY` is created by the campaign runner (at least 16
random bytes), held privately until both manifests have been compared, then
removed. Both legs and the comparator must use the same campaign key. It is
never installed in the kernel and cannot authenticate a WebSocket connection.

An ordinary loopback kernel may be started without local bearer authentication.
If configured, the existing client accepts `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN`
or its one-shot `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE`, bound to the exact
loopback endpoint. The collector consumes it through the normal client and
passes the admitted context only to its owned probes, keeping it out of Git,
provider-version commands and evidence. It does not mint a kernel bearer.

Path-1 bootstrap creates a random local bearer under
`$CHARIOX_HOME/managed-runtime-auth/`; the kernel consumes and unlinks that file
before launching providers. Do not extract or inherit this bootstrap credential.
Use an already authenticated controller's ordinary
`ResolveKernelClientConnection { kernel_ref: <exact-kernel-id> }` request to
obtain an account- or machine-authorized target-scoped relay connection. This
uses the existing Cloud token issuer and relay admission. Supply the returned
`relay_url`, `relay_token`, and exact `kernel_id` through the approved private
capture context named above. Never print the response or put its token in a
command, prompt, URL, or evidence. A missing approved context is an admission
prerequisite failure, not an instruction to disable loopback authentication.
`IssueCloudRelayClientToken` also supports key-bound tokens, but the current
collector creates independent ephemeral relay keys per client: such a token
cannot be substituted without sharing its normal client identity with every
observer. Use the normal resolved connection contract for this collector.

Product-route process binding requires local protocol 371's
`RelayStatus.runtime_process_identity`. Boot ID, PID, and process start ticks
must agree with the independent Linux observation before and after collection.
Loopback captures also verify TCP listener ownership. Legacy or non-Linux
status without this optional field fails product-route capture rather than
substituting caller-supplied process identity. The existing Unix IPC capture
remains supported for kernels that actually expose that transport. Neither a
source-test pass nor a transport-only probe supplies missing acceptance rows.

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
machine-readable list of required rows and checks. Its identifiers match the
locked plan ledger exactly:

| Row | Runtime evidence |
| --- | --- |
| `MP-01` | Ordinary provider launch: ancestry, managed-isolation environment, mounts, privileges, network, and permitted tool/package installation. |
| `MP-02` | Directory discovery, exact-path entry, creation, `/home`, and `/tmp`. |
| `MP-03` | Exact managed control-file protection and ordinary filesystem permissions. |
| `MP-04` | Ordinary `HOME`, `CHARIOX_HOME`, cwd, and worker-user environment. |
| `MP-05` | Empty workspace, copied repository, preserved basename, collision rejection, and worktree placement. |
| `MP-06` | Default/custom server-authoritative repository root, inheritance, and client-override rejection. |
| `MP-07` | Signed immutable managed release, atomic activation, rollback, and reviewed source identity. |
| `MP-08` | Provider, session, terminal, file, Git, attachment, permission, Project setup, reconnect, restart, history, queued/active turn, resource, error, protocol, and cleanup parity. |
| `MP-09` | Every mandatory managed automatic-shutdown policy and reconciliation trigger. |
| `MP-10` | Reviewed source/protocol identity, fresh worker, and authorized capture boundary. |

The plan-level `MP-11` source inventory is a separate proactive audit gate and
cannot be represented by a runtime snapshot alone. `MP-01/provider_ancestry`
must prove no Bubblewrap ancestor on Path 1, and
`MP-01/managed_isolation_environment` must prove the managed isolation and
Bubblewrap environment markers are absent.

The shutdown rows are mandatory even when a policy means that automatic stop is
not expected. They cover `agents_done`, 15-minute idle, 30-minute idle, minimum
three-hour runtime, disabled mode, keep-running mode, restart reconciliation,
manual stop, custom delay, explicit lifecycle reconciliation, and deployment
reconciliation. Each row records the exact expected and observed outcome,
whether the worker stopped, cleanup confirmation, the configured delay, the
observed delay, and whether timing was measured from the last agent finishing.
Arbitrary nonempty outcome strings, premature deadlines, excessive delay, or a
stop in disabled/keep-running mode fail closed.

The focused Node tests contain green parity, tamper, missing-row,
topology/head mismatch, different-result, cleanup-failure, invalid shutdown
outcome, premature deadline, and disabled-mode negative fixtures. They validate
the collector/comparator contract only. A successful focused test run does not
replace live capture from an ordinary Linux kernel and a fresh Path-1 worker;
that live execution remains required before parity is accepted.
