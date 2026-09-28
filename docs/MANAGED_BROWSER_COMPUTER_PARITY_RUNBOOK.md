# CHA-16 Managed Browser/Computer Parity Runbook

This attended acceptance procedure targets one fresh, drill-owned
OpenShip-managed machine. It is not a provisioner: provisioning,
authentication, deployment, restart, and teardown remain explicit operator
actions. Never run it against shared or production infrastructure.

The orchestrator is fail-closed. Passing evidence requires the signed Selkies
product default. Explicit noVNC is rollback coverage and can never make the run
pass. Web runtime traffic remains `browser -> hosted relay -> selected kernel`;
Cloud supplies authenticated bootstrap and metadata only and never proxies
Browser, Computer, prompt, provider, Git, or vault payloads.

## Prerequisites

1. A fresh OpenShip machine with no prior drill Room, slice, browser profile,
   or active kernel target.
2. The exact managed image `sha256:` digest, exact base64 Ed25519 signature,
   pinned signer public-key fingerprint, and a successful release verifier
   result. Never copy a signing private key to the target or config.
3. Exact clean OSS and Cloud 40-character SHAs. The image must identify the OSS
   SHA and the attended Web deployment must identify the Cloud SHA.
4. A kernel compatible with the released adapter and client, exact relay
   protocol and released relay version, and a fresh heartbeat for the expected
   immutable kernel/machine. The complete Path-1 flow additionally requires
   protocol 351 home controls; an older browser-only capability minimum does
   not admit the complete flow.
5. Released Web, local TUI, and remote TUI clients, authenticated through their
   normal product paths. Do not pass their cookies or tokens to this harness.
6. Codex, OpenCode, and Claude advertised through official provider harnesses.
   The drill checks capability presence and never copies provider-internal state.
7. Product-managed Git authentication and an attended, synthetic-test vault
   entry. The adapter receives only fixture name `synthetic-vault-marker-v1`,
   never its value.
8. A reviewed adapter module exporting
   `MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY` and
   `createManagedBrowserComputerParityTransport`. Its exact regular-file bytes
   are SHA-256 pinned in the private config and verified before import; a
   missing, symlinked, hash-mismatched, or self-asserted module fails closed.
   Construction must be side-effect free. Its steps must use the released
   BrowserKernelClient, local IPC, and remote relay paths—not Cloud runtime APIs
   or another authority—and return authoritative kernel, machine, Room,
   environment, and backend IDs. The factory receives only the external
   evidence directory. Preflight must independently inspect the installed
   image/source/protocol/target rather than echo expected config values;
   expected identity is supplied only to bound product and cleanup steps.
9. A separately reviewed cleanup-inspector module exporting
   `MANAGED_BROWSER_COMPUTER_PARITY_INSPECTOR_IDENTITY` and
   `createManagedBrowserComputerParityInspector`. Its exact bytes and identity
   are pinned under `inspector` below. The inspector must be independent of the
   product transport, use only authoritative cleanup inventories, and fail
   closed on a mismatched loader proof or target identity.
10. Conservative ceilings for RSS, CPU, free memory/disk, heartbeat age, and
   post-cleanup RSS/disk deltas, with enough reserve to complete cleanup.

### Storage and signing-key retention

Keep source worktrees free of build outputs and runtime state. Use an explicit
remote build-output directory and check its free-space reserve before and
during builds. Record the exact process or unit that owns each temporary
directory; settle it before cleanup. Retain small result receipts separately
from rebuildable compiler caches, archives, and image exports.

Release and builder private signing keys are durable credentials, not build
artifacts. Store them outside disposable task/build directories with mode0600,
and maintain a separately protected backup. Record only public fingerprints in
evidence. Before artifact cleanup, inventory retained credentials and trust
pins explicitly; never recursively delete their parent as build scratch.
Recovery must verify the original public identity before signing. If the
private signer is lost, stop signing and obtain an explicit trust-rotation
decision. A public key or an unrelated development signer cannot replace it.

Create a regular mode-0600 metadata JSON file outside both repositories. It
contains no credential or secret value:

```json
{
  "runId": "cha-16-YYYYMMDD-HHMMSS",
  "ossSha": "<40 lowercase hex>",
  "cloudSha": "<40 lowercase hex>",
  "adapter": {
    "identity": "<reviewed adapter identity>",
    "sha256": "sha256:<64 lowercase hex>"
  },
  "inspector": {
    "identity": "<reviewed independent inspector identity>",
    "sha256": "sha256:<64 lowercase hex>",
    "observations": {
      "cloudInventoryModule": {
        "path": "<absolute reviewed read-only Cloud inventory module path>",
        "identity": "<reviewed Cloud inventory identity>",
        "sha256": "sha256:<64 lowercase hex>"
      },
      "host": "<SSH host alias for the worker>",
      "cloudInventory": {
        "accountId": "<owning Cloud account>",
        "realmId": "<owning relay realm>",
        "userId": "<user requesting normal managed DELETE>"
      },
      "machineOwnership": {
        "kind": "run_owned",
        "machineId": "<immutable managed machine id>",
        "creationReceipt": {
          "runId": "cha-16-YYYYMMDD-HHMMSS",
          "machineId": "<immutable managed machine id>",
          "kernelId": "<immutable kernel id>",
          "managedEnvironmentId": "<Cloud managed environment id>",
          "providerServerId": "<provider server id>",
          "createOperationId": "<normal CREATE operation id>"
        }
      }
    }
  },
  "image": {
    "digest": "sha256:<64 lowercase hex>",
    "signature": "<exact base64 Ed25519 release signature>",
    "signerFingerprint": "sha256:<64 lowercase hex>",
    "sourceTree": "<40 lowercase hex Git tree>",
    "target": "x86_64-unknown-linux-gnu"
  },
  "expected": {
    "kernelId": "<immutable kernel id>",
    "machineId": "<immutable managed machine id>",
    "roomId": "<drill-owned Room id>",
    "environmentId": "<drill-owned environment id>"
  },
  "resourceCeilings": {
    "maximumRssBytes": 2147483648,
    "maximumCpuPercent": 300,
    "minimumFreeMemoryBytes": 2147483648,
    "minimumFreeDiskBytes": 10737418240,
    "maximumHeartbeatAgeMs": 15000,
    "maximumPostRunRssDeltaBytes": 52428800,
    "maximumPostRunDiskDeltaBytes": 10485760
  },
  "stepTimeoutMs": 600000
}
```

The inventory module must export `createManagedParityCloudInventory` and declare
the reviewed module identity required by the inspector loader. Pin its current
hash together with the inspector; an older module without retirement evidence
cannot validate run-owned cleanup. Use `preexisting` ownership only for an
explicitly retained machine, never to avoid run-owned deletion proof.

For run-owned cleanup, `retirementProof.authority` must be
`normal-managed-delete`. Bind account, realm, user, environment, machine, kernel
and CREATE operation to the metadata above. Include the successful normal DELETE
operation, its deterministic `parity-cleanup-<sha256(runId)>` idempotency key,
completed time and desired revision; the DELETED environment must have observed
that revision. The machine must be REVOKED with leases disabled and a revocation
time. Census counts `activeCredentials`, `unrevokedTokens`, `unrevokedGrants` and
`nonRevokedTargets` must all be zero. Include one
`managed_environment_deleted` tombstone for the machine and each owned kernel.
These database observations do not replace independent provider absence or
physical process, volume and profile cleanup checks.

`retainedHeartbeatRows` projects heartbeat data from retained relay target rows,
not a separate heartbeat identifier namespace. Its `id` must be the target row
ID, `kernelId` must equal that target's `daemonId`, and `lastHeartbeatAt` must
match exactly. Record those target IDs in `ownedHeartbeatIds` and
`queriedHeartbeatIds`. Retained targets must be REVOKED and scoped to the exact
account, realm, machine and owned kernels. Preserve historical counts in the
evidence rather than pretending revoked history was physically deleted.

## Exact live command

Run only after reviewing every prerequisite. Arguments contain metadata paths;
authentication stays in the already-authenticated product clients:

```sh
node apps/cli/scripts/live-managed-browser-computer-parity-drill.mjs \
  --config /absolute/private/cha-16-managed-parity.json \
  --transport-module /absolute/reviewed/managed-parity-product-transport.mjs \
  --inspector-module /absolute/reviewed/managed-parity-cleanup-inspector.mjs \
  --evidence-root /absolute/external/evidence/browser-computer-use/cha-16
```

Never place a credential, token, cookie, vault value, provider profile, prompt,
or secret in arguments, environment overrides, diagnostics, evidence, or
fixtures. The adapter uses only existing authenticated product stores.

## Orchestrated product sequence

After verifying signed image/source/protocol/relay/target metadata, heartbeat,
capabilities, resource headroom, and the separately pinned inspector authority,
the orchestrator:

1. Creates the headed environment through the kernel-owned default. The result
   must be `selkies` with exactly one Room, browser, and profile.
2. Attaches Web, local TUI, and remote TUI to that same Room/environment.
3. Confirms official Codex, OpenCode, and Claude harness capabilities.
   Evidence must explicitly report that no provider-internal state was copied.
4. Runs structured Browser discovery/fill/click with one mutation, then Computer
   screenshot, pointer, and keyboard input.
5. Verifies actor overlay, human takeover/cancellation, and attribution.
6. Saves and restarts, preserving Room, environment, profile, and provider thread.
7. Inserts the synthetic marker only through kernel secret input and requires
   zero matches in arguments, logs, evidence, prompts, and fixtures.
8. Confirms Git auth availability without reading its material.
9. Injects a bounded relay disconnect, reconnects without stale identity, and
   requires zero duplicate actions or browsers.
10. Destroys Selkies, creates the explicit noVNC rollback, attaches all three
    clients to the same bound identity, records it as rollback-only, and destroys it.

Old/unknown protocol, unsigned image, stale heartbeat, target rebind, missing
capability, duplicate authority, partial result, timeout, or a noVNC acceptance
claim fails the run.

## Cleanup and evidence

Cleanup runs exactly once after success, failure, or timeout. The final inventory
is obtained through the independently pinned cleanup inspector, never by
delegating inspection back to the product transport. Remove only
drill-owned OpenShip/Cloud records, Room/environment/slice state, containers,
volumes, networks, processes, listeners, profiles, tunnels, temporary files,
and synthetic grants. Never prune shared resources.

The independent cleanup inspection must report zero managed machines, Rooms,
environments, processes, listeners, containers, profiles, active relay targets
and heartbeats, temporary files, and evidence leaks. RSS/disk deltas must fit
their ceilings. Cleanup command success without a clean inventory fails.

After the report and `chariox-drill-artifacts.json` are written, the CLI physically
enumerates the complete run directory with bounded file/byte/depth limits. It
rejects symlinks, special files, secret-looking names or values, and emits the
checksummed manifest as a private sibling of the run directory so the manifest
does not enumerate itself. A run is not accepted when this final scan fails.

Retain `managed-browser-computer-parity.json`, checksummed
`chariox-drill-artifacts.json`, and the private sibling evidence manifest
outside repositories. Review the recorded image signature/digest, SHAs,
protocol/relay versions, identity, resources, steps, and cleanup before deleting
the private metadata config.
