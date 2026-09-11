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
4. Kernel protocol 322 or newer, exact relay protocol and released relay
   version, and a fresh heartbeat for the expected immutable kernel/machine.
5. Released Web, local TUI, and remote TUI clients, authenticated through their
   normal product paths. Do not pass their cookies or tokens to this harness.
6. Codex, OpenCode, and Claude advertised through official provider harnesses.
   The drill checks capability presence and never copies provider-internal state.
7. Product-managed Git authentication and an attended, synthetic-test vault
   entry. The adapter receives only fixture name `synthetic-vault-marker-v1`,
   never its value.
8. A reviewed adapter module exporting
   `createManagedBrowserComputerParityTransport`. Construction must be
   side-effect free. Its steps must use the released BrowserKernelClient, local
   IPC, and remote relay paths—not Cloud runtime APIs or another authority—and
   return authoritative kernel, machine, Room, environment, and backend IDs.
   The factory receives only the external evidence directory. Preflight must
   independently inspect the installed image/source/protocol/target rather than
   echo expected config values; expected identity is supplied only to bound
   product and cleanup steps.
9. Conservative ceilings for RSS, CPU, free memory/disk, heartbeat age, and
   post-cleanup RSS/disk deltas, with enough reserve to complete cleanup.

Create a regular mode-0600 metadata JSON file outside both repositories. It
contains no credential or secret value:

```json
{
  "runId": "cha-16-YYYYMMDD-HHMMSS",
  "ossSha": "<40 lowercase hex>",
  "cloudSha": "<40 lowercase hex>",
  "image": {
    "digest": "sha256:<64 lowercase hex>",
    "signature": "<exact base64 Ed25519 release signature>",
    "signerFingerprint": "sha256:<64 lowercase hex>"
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

## Exact live command

Run only after reviewing every prerequisite. Arguments contain metadata paths;
authentication stays in the already-authenticated product clients:

```sh
node apps/cli/scripts/live-managed-browser-computer-parity-drill.mjs \
  --config /absolute/private/cha-16-managed-parity.json \
  --transport-module /absolute/reviewed/managed-parity-product-transport.mjs \
  --evidence-root /absolute/external/evidence/browser-computer-use/cha-16
```

Never place a credential, token, cookie, vault value, provider profile, prompt,
or secret in arguments, environment overrides, diagnostics, evidence, or
fixtures. The adapter uses only existing authenticated product stores.

## Orchestrated product sequence

After verifying signed image/source/protocol/relay/target metadata, heartbeat,
capabilities, and resource headroom, the orchestrator:

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

Cleanup runs exactly once after success, failure, or timeout. Remove only
drill-owned OpenShip/Cloud records, Room/environment/slice state, containers,
volumes, networks, processes, listeners, profiles, tunnels, temporary files,
and synthetic grants. Never prune shared resources.

The independent cleanup inspection must report zero managed machines, Rooms,
environments, processes, listeners, containers, profiles, active relay targets
and heartbeats, temporary files, and evidence leaks. RSS/disk deltas must fit
their ceilings. Cleanup command success without a clean inventory fails.

Retain only `managed-browser-computer-parity.json` and checksummed
`chariox-drill-artifacts.json` outside repositories. Review the recorded image
signature/digest, SHAs, protocol/relay versions, identity, resources, steps, and
cleanup before deleting the private metadata config.
