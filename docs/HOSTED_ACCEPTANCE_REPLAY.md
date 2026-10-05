# MP-01..MP-11 — exact-source acceptance replay commands

Companion to [OWNER handoff](HOSTED_ACCEPTANCE_OWNER_HANDOFF.md). These are
command templates bound to OSS `358491d662330aff538bbd6d6450391332f62ac2`.
Set public identifiers/absolute paths from admitted receipts. Unknown values
are prerequisites, never guessed defaults. Recheck every command against the
final reviewed aggregate before execution. Live commands are OWNER-only;
b219 ran only the offline helper tests and usage checks below.

## MP-07/MP-08/MP-10/MP-11 — freeze and offline review

Run from the admitted checkout, with public variables `OSS_CHECKOUT`,
`OSS_COMMIT`, `OSS_TREE`, `CLOUD_CHECKOUT`, `CLOUD_COMMIT`, `CLOUD_TREE`, and
`EVIDENCE` supplied. `EVIDENCE` is a fresh absolute external directory. On this
builder it belongs below `/root/.codex/evidence/browser-resume-20260930/b219/`.
No credential/profile/config file is an input to these checks.

```bash
# MP-07/MP-08/MP-10/MP-11: read-only identity checks, no fetch/publish/deploy.
umask 077
set -o noclobber
test "$(git -C "$OSS_CHECKOUT" rev-parse HEAD)" = "$OSS_COMMIT"
test "$(git -C "$OSS_CHECKOUT" rev-parse 'HEAD^{tree}')" = "$OSS_TREE"
git -C "$OSS_CHECKOUT" diff --exit-code
git -C "$OSS_CHECKOUT" diff --cached --exit-code
git -C "$OSS_CHECKOUT" status --porcelain
test "$(git -C "$CLOUD_CHECKOUT" rev-parse HEAD)" = "$CLOUD_COMMIT"
test "$(git -C "$CLOUD_CHECKOUT" rev-parse 'HEAD^{tree}')" = "$CLOUD_TREE"
git -C "$CLOUD_CHECKOUT" status --porcelain
node "$OSS_CHECKOUT/apps/cli/scripts/managed-ordinary-parity-collector.mjs" --help
node "$OSS_CHECKOUT/apps/cli/scripts/managed-ordinary-parity-matrix.mjs" --help
```

MP-10/MP-11: any porcelain output keeps the freeze open, including untracked
code. Cross-repository PR descriptions must record exact paired heads. Bind
each independent review to SHA, true provider/model/account-role/effort and
all findings' disposition/new-commit mapping. Check the lane REVIEW_INBOX after
each commit batch. Missing/stale/incomplete/falsely attributed review is open.
Coordinator requests final CI after focused checks and exact-head clean review;
b219 does not dispatch it. Inspect `.github/workflows/ci.yml` at the candidate:
frozen OSS gates on opened non-draft/ready-for-review or explicit dispatch,
not synchronize. Record tested checkout SHA as well as workflow/event SHA.
Include selected Apps/native/Chromium Linux/macOS workflows and Cloud checks
from their real impacted paths; do not infer a universal green result from G2.

```bash
# MP-07/MP-10/MP-11: offline fixtures; no actual Cloud/provider capture.
node --test --test-concurrency=1 \
  apps/cli/scripts/path1-host-cloud-correlation.test.mjs \
  apps/cli/scripts/path1-provider-cloud-correlation.test.mjs

# MP-11: exact-source scan; nonzero exit/unreviewed dispositions remain RED.
node apps/cli/scripts/managed-parity-source-inventory.mjs \
  --root "$OSS_CHECKOUT" --source-ref "$OSS_COMMIT" \
  --expect-source-commit "$OSS_COMMIT" --expect-source-tree "$OSS_TREE" \
  --output "$EVIDENCE/oss-inventory.json"
node apps/cli/scripts/managed-parity-source-inventory.mjs \
  --root "$CLOUD_CHECKOUT" --source-ref "$CLOUD_COMMIT" \
  --expect-source-commit "$CLOUD_COMMIT" --expect-source-tree "$CLOUD_TREE" \
  --output "$EVIDENCE/cloud-inventory.json"
```

MP-11 scan commands are prepared, not executed by b219. Independent semantic
review and installed policy are required beyond an inventory with zero gaps.

## MP-01..MP-11 — local-to-hosted preparation

Set an explicit external absolute `CHARIOX_HOME` for each newly owned runtime,
for example a product-created campaign directory below
`~/.chariox/dev/b219/`; never a repository or a shared/default kernel home.
Use only product-linked provider profiles and documented materialization.
Do not read or copy raw provider-account files. A provider 401/unauthorized
stops that provider cell and must be reported immediately to the coordinator.

```bash
# MP-08/MP-10/MP-11: owned slice provisioner for a future approved live run.
export CHARIOX_SLICE_DOCKER_PROVISIONER="$OSS_CHECKOUT/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"
# MP-08/MP-10: deterministic M20 command with admitted prebuilt artifacts.
# This frozen script tests the slice browser; it does not prove host Browser persistence.
M20_USE_PREBUILT=1 M20_KERNEL_BINARY="$KERNEL_BINARY" M20_SLICE_IMAGE="$SLICE_IMAGE" \
  M20_ARTIFACT_DIR="$EVIDENCE/m20" M20_RUNTIME_ROOT="$M20_STATE" \
  pnpm --dir "$OSS_CHECKOUT/apps/cli" browser-computer:persistence-drill
```

MP-08/MP-10: run M20 only after matching binary/image/client/source provenance,
with owned external state/evidence, source-valid dependencies and resource
admission. Its old backend assertion needs reviewed replacement applicability.
`M20_STATE` is a fresh absolute lane-owned directory under `~/.chariox/dev/b219/`,
outside repositories; inventory and remove disposable state after the run.
On a host-port collision remove/recreate only the owned slice and retry at most
three times, retaining each attempt. Do not add a shared port lock.

MP-07/MP-10: private Cloud bundle must supply the pinned OpenShip deployment,
fresh machine create/reimage and product enrollment commands with approved
cost/lifetime. This lane cannot supply a verified deployment CLI while the
bundle is absent. OWNER uses the reviewed local home kernel's waiting-room
managed-machine Create/Reimage/Start controls, then the actual Web/TUI flow.
Stop here if product commands, signed target image or authorization are missing.
Do not install via pnpm, hand-edit service units, touch staging from this lane,
or contact protected relay/Apps machines.

## MP-07/MP-10/MP-11 — installed signed release and fresh host capture

On the authorized target, `RELEASE_ROOT` must be the actual installed
`/usr/lib/chariox/releases/<64-hex-manifest-digest>` with expected mode/ownership.
Portable builder `.../portable/rootfs` is not an acceptable installed layout.
Use independently approved external **public** pins as `RELEASE_PUBLIC_PIN`
and `BUILDER_PUBLIC_PIN`; source them from the owner's approved inventory.
No release private keys travel to the target or builder.

```bash
# MP-07/MP-10/MP-11: verifies signatures/artifacts against approved public pins.
node "$OSS_CHECKOUT/apps/cli/scripts/path1-reviewed-release-capture.mjs" \
  --release-root "$RELEASE_ROOT" \
  --trusted-release-public-key "$RELEASE_PUBLIC_PIN" \
  --trusted-builder-public-key "$BUILDER_PUBLIC_PIN" \
  --expected-source-commit "$OSS_COMMIT" --expected-source-tree "$OSS_TREE" \
  --executable chariox-kernel --evidence-output "$EVIDENCE/reviewed-kernel.json"

# MP-01/MP-04/MP-07/MP-10/MP-11: on approved worker only; public status fields.
# Use distinct before/after filenames around an approved reimage.
node "$OSS_CHECKOUT/apps/cli/scripts/path1-host-identity-capture.mjs" \
  --unit chariox-path1-managed-bootstrap.service \
  --unit chariox-rootless-docker.service \
  --unit chariox-slice-broker.service > "$EVIDENCE/host-after.json"
node "$OSS_CHECKOUT/apps/cli/scripts/path1-host-residue-capture.mjs" \
  --before "$EVIDENCE/host-before.json" --output "$EVIDENCE/host-residue.json"
```

MP-07/MP-10: ensure the redirected host output is a new private evidence file;
the identity command emits JSON to stdout and does not accept `--output`.
Do not run worker captures on the reserved builder. Fresh allocation has its
own receipt; the following reimage commands apply only to approved reuse.

```bash
# MP-07/MP-10: normal local home-kernel read, after finalized product reimage.
# HOME_KERNEL_URL is explicit ws://127.0.0.1:PORT/kernel, not hosted wss.
node "$OSS_CHECKOUT/apps/cli/scripts/path1-cloud-reimage-capture.mjs" \
  --kernel "$HOME_KERNEL_URL" --environment "$ENVIRONMENT_ID" \
  --operation "$OPERATION_ID" --generation "$GENERATION" \
  --release "$RELEASE_DIGEST" --commit "$OSS_COMMIT" --tree "$OSS_TREE" \
  --output "$EVIDENCE/cloud-reimage.json"
node "$OSS_CHECKOUT/apps/cli/scripts/path1-fresh-relay-capture.mjs" \
  --kernel "$HOME_KERNEL_URL" --machine "$MACHINE_REF" \
  --output "$EVIDENCE/fresh-relay.json"
node "$OSS_CHECKOUT/apps/cli/scripts/path1-host-cloud-correlation.mjs" \
  --before "$EVIDENCE/host-before.json" --after "$EVIDENCE/host-after.json" \
  --cloud "$EVIDENCE/cloud-reimage.json" --output "$EVIDENCE/host-cloud-correlation.json"
node "$OSS_CHECKOUT/apps/cli/scripts/path1-provider-cloud-correlation.mjs" \
  --before "$EVIDENCE/provider-before.json" --after "$EVIDENCE/provider-after.json" \
  --cloud "$EVIDENCE/cloud-reimage.json" --output "$EVIDENCE/provider-cloud-correlation.json"
```

MP-07/MP-10: provider captures are owner-side read-only provider-API receipts,
not this builder's authorization to extract credentials or call cloud APIs.
`path1-provider-rebuild-capture.mjs before --server-id ... --output ...` and
`after --server-id ... --image-id ... --action-id ... --requested-at ... --output ...`
are available to the owner's admitted operator context. The Cloud capture needs
the built shared client and finalized generation; pending/wrong bindings fail.
If intentionally retaining the source controller relay realm, use its supported
`--shared-controller-target` only with the verified source binding. Old worker
identity/credential/target/heartbeat retirement still remains mandatory.

MP-10: the final rebuild verifier consumes reviewed/before/rebuild/after/cleanup
receipt envelopes with observation evidence references/hashes. Raw captures
are not those envelopes. Coordinator must supply the reviewed assembler and
authentic provider/relay/cleanup observations; never hand-label captures PASS.

```bash
# MP-07/MP-10/MP-11: offline verification of authentic assembled envelopes.
node "$OSS_CHECKOUT/apps/cli/scripts/live-path1-rebuild-evidence-verifier.mjs" \
  --reviewed "$EVIDENCE/reviewed-envelope.json" --before "$EVIDENCE/before-envelope.json" \
  --rebuild "$EVIDENCE/rebuild-envelope.json" --after "$EVIDENCE/after-envelope.json" \
  --cleanup "$EVIDENCE/cleanup-envelope.json" --report "$EVIDENCE/rebuild-report.json"
```

## MP-01..MP-10 — paired official-provider collector

OWNER supplies admitted runtime/auth/setup context and the collector HMAC
environment through its approved mechanism. `CHARIOX_PARITY_SIGNING_KEY` is
required by the existing collector/comparator; it is not a release signature.
Do not generate a substitute owner key, print it, copy it manually, store it in
evidence, or claim HMAC fixtures verify release trust. Missing admitted context
blocks this command. Run inside the actual provider cwd as the matching user;
repeat sequentially for ordinary/path1 and Codex/Claude/OpenCode.

```bash
# MP-01..MP-10: target-bound real provider capture; public path/identity arguments.
node "$OSS_CHECKOUT/apps/cli/scripts/managed-ordinary-parity-collector.mjs" capture \
  --topology "$TOPOLOGY" --reviewed-commit "$OSS_COMMIT" --build-id "$BUILD_ID" \
  --kernel-protocol "$LOCAL_PROTOCOL" --relay-protocol "$RELAY_PROTOCOL" \
  --provider "$PROVIDER" --provider-command "$OFFICIAL_PROVIDER_COMMAND" \
  --kernel-binary "$KERNEL_BINARY" --kernel-release-root "$RELEASE_ROOT" \
  --kernel-release-digest "$RELEASE_DIGEST" \
  --kernel-release-public-key "$RELEASE_PUBLIC_PIN" \
  --kernel-builder-public-key "$BUILDER_PUBLIC_PIN" --boundary official-provider-turn \
  --source-root "$OSS_CHECKOUT" --expected-cwd "$PROVIDER_CWD" \
  --output "$EVIDENCE/$TOPOLOGY-$PROVIDER.json" \
  --signing-key-env CHARIOX_PARITY_SIGNING_KEY
node "$OSS_CHECKOUT/apps/cli/scripts/managed-ordinary-parity-matrix.mjs" compare \
  --ordinary "$EVIDENCE/ordinary-$PROVIDER.json" \
  --path1 "$EVIDENCE/path1-$PROVIDER.json" --report "$EVIDENCE/parity-$PROVIDER.json" \
  --reviewed-commit "$OSS_COMMIT" --build-id "$BUILD_ID" \
  --signing-key-env CHARIOX_PARITY_SIGNING_KEY
```

## MP-08/MP-09/MP-10 — hosted product and lifecycle replay

Use actual reviewed transport/inspector modules and a private product-created
config. Inspect only allowed public status fields, never dump the config.
This harness pins adapter hashes/identities; arbitrary mocks cannot establish
hosted Web/TUI/provider acceptance.

```bash
# MP-08/MP-10: real provider/client persistence adapters, after local gates.
node "$OSS_CHECKOUT/apps/cli/scripts/live-managed-browser-computer-parity-drill.mjs" \
  --config "$PRIVATE_CAMPAIGN_CONFIG" --transport-module "$TRANSPORT_MODULE" \
  --inspector-module "$INSPECTOR_MODULE" --evidence-root "$EVIDENCE/hosted"

# MP-09/MP-10: creates/deletes ONE approved agent-small target per scenario.
# Interactive OWNER required for product UI actions; command is not read-only.
node "$OSS_CHECKOUT/apps/cli/scripts/live-managed-shutdown-trigger-drill.mjs" \
  --scenario "$SHUTDOWN_SCENARIO" --kernel-url "$HOME_KERNEL_URL" \
  --region "$APPROVED_REGION" --compute-class agent-small \
  --max-billable-seconds "$APPROVED_MAX_SECONDS" \
  --confirm-one-target CREATE-AND-DELETE-ONE-MANAGED-TARGET \
  --output "$EVIDENCE/$SHUTDOWN_SCENARIO.json"
```

MP-09 supported scenario IDs: `shutdown_agents_done`, `shutdown_idle_15m`,
`shutdown_idle_30m`, `shutdown_minimum_3h`, `shutdown_disabled`,
`shutdown_keep_running`, `shutdown_restart_reconciliation`,
`shutdown_all_clients_disconnected`, `shutdown_manual`, `shutdown_custom`,
`shutdown_explicit_lifecycle_reconciliation`, `shutdown_deployment_reconciliation`.
Caps are scenario-dependent (600–12600 seconds minimum, 14400 maximum in
frozen parser). The no-ACK 3600-second stale-heartbeat policy and real provider
STOP/disk verification require the B2-14 reviewed campaign; this parser does
not expose a no-ACK scenario. Do not shorten minimum-runtime or no-ACK proof.

MP-08/MP-10 remaining exact command prerequisites: g2fix native/provider
matrix, b207 real model-visible byte/Web/TUI conjunction, kbrowser host profile
and native Mac replay, Apps/appviews/displays, RTO/RPO faults and replacement
8h/24h runner. Obtain each lane's source-bound entry point before execution.
Run Drills A–I using actual client actions and first-party tools; the historical
`browser-computer:soak --detach` cannot validate the new transport. Do not
launch another Selkies-only soak or apply the 377/69 retirement patch.

## MP-01..MP-11 — evidence and owned cleanup

Record exact command, cwd, source/tree, binary/image/signatures/client/provider/
browser/OS versions, public machine/boot/operation/generation IDs, start/end,
exit/first failure, before/during/after resources, actor/target/action/thread
invariants and cleanup. Track S/T1/T2/T3/T4 scope explicitly. Leak scanner
canaries stay private outside evidence; scan only admitted artifacts with the
reviewed scanner, publish redacted result counts and no secret material.

Inventory exact owned IDs/names/labels before deletion. On success/failure/
timeout/interruption settle owned processes, ports, sessions, containers,
temporary volumes/transfers/state/runtime keys and Cloud/provider resources.
Validate archived persistence before removal. Protect durable product-linked
credential profiles, retained homes/saved copies, key stores/backups and shared
reviewer state. Do not recursively delete a parent containing those assets or
prune Docker caches. Final receipt proves no owned active target/heartbeat,
temporary secret file, process/listener or billable resource remains and records
recovered memory/disk plus explicit protected-retention reasons.
