# Project environment setup acceptance drill

`apps/cli/scripts/live-project-environment-setup-acceptance-drill.mjs` is the strict live Path1 acceptance drill. It is an operator drill, not a fixture or a readiness claim. This authoring change does not execute it.

The drill uses the public `LocalIpcClient` transport and the official kernel request builders. The home kernel must create a session with `kernel_ref` set to the selected worker kernel. That public `CreateSession` composition produces the remote-backed agent used by `StartProjectEnvironmentSetup`; the drill never injects a lease, worker receipt, provider result, or runtime binding.

## Acceptance sequence

The drill fails closed unless all of these are true:

- the endpoint is a WebSocket home-kernel or relay endpoint and the event transport is available;
- a read-only `GetProjectEnvironmentSetupStatus` probe reaches the selected home kernel, proving protocol 326 support. An `unknown variant` response is reported as an explicit minimum-version failure, while unrelated errors remain unverified;
- the exact worker machine is approved, online, not pending, and advertises the selected provider/account; the exact worker kernel belongs to that machine, accepts remote leases, and the created agent exposes relay peer protocol 51 or newer;
- an account-scoped Cloud `GET /disposable-workers/:allocationId?accountId=...` returns the selected DisposableWorkerAllocation. Its allocation ID, `homeKernelId` (matched to the connected home `RelayStatus`), `homeRelayRealmId` (matched to the authenticated Cloud profile), runtime machine/kernel IDs, `desiredState`/`observedState`, equal desired/observed revisions, future `expiresAt`, and exact `runtimeReleaseDigest` (`sha256:` plus 64 lowercase hex) must all match. The digest is an allocation release identity, not continuous attestation of a currently running binary;
- the selected active Project exists, belongs to the selected workspace, has no existing environment definition, and the created session preserves the exact Project, workspace, and worktree;
- at least one real bounded validation command is supplied.

After preflight, the drill performs:

1. `StartProjectEnvironmentSetup` without a definition, with the requested validation commands. Kernel-owned utility execution derives the definition on the worker through the official provider path.
2. `GetProjectEnvironmentSetupStatus` polling. By default the drill requires an observed worker `preparing` or `validating` phase, then sends `CancelProjectEnvironmentSetup` and `RetryProjectEnvironmentSetup` and requires a new attempt before continuing. `--allow-terminal-before-cancel` is an explicit exception for a setup that became terminal before cancellation was applicable and records that gap in the report.
3. More `GetProjectEnvironmentSetupStatus` polling until authoritative `ready`. The status must identify the exact worker/platform and contain a non-empty, all-zero-exit validation result set. The Project is then re-read and must contain a utility-generated definition with non-zero setup/install steps and the requested validation commands.
4. Only after the observed `ready` status, one public `LaunchProviderRun`, one exact `SubmitPrompt`, and one matching `assistant_message_completed` event. The request log records the ordering and the provider run identity without recording prompt text, command text, provider output, or credentials.
5. Explicit event unsubscribe, attachment detach, and end-session cleanup for only the drill-created session. The persisted Project definition is intentionally retained as the user-owned setup result; the selected Project, worktree, disposable worker allocation, worker, and resource caps are never deleted or mutated by the drill.

## Invocation

Build the normal kernel-client package before a live run so `packages/kernel-client/dist/ipc.js` and `dist/ipc-requests.js` are present. The drill itself never builds them. Use existing deployed inputs; do not provision a machine or change credentials.

For a relay-connected home kernel, keep the token in the environment rather than the command line:

```sh
export CHARIOX_RELAY_TOKEN='provided-by-the-existing-runtime'
export CHARIOX_PATH1_EXPECTED_RUNTIME_RELEASE_DIGEST='sha256:<64 lowercase hex characters from the approved release>'
pnpm --filter @chariox/cli run project-environment:setup-acceptance -- \
  --relay-url "$CHARIOX_RELAY_URL" \
  --home-daemon-id "$CHARIOX_HOME_DAEMON_ID" \
  --home-kernel-id "$CHARIOX_PATH1_HOME_KERNEL_ID" \
  --worker-machine-id "$CHARIOX_PATH1_WORKER_MACHINE_ID" \
  --worker-kernel-id "$CHARIOX_PATH1_WORKER_KERNEL_ID" \
  --disposable-worker-allocation-id "$CHARIOX_PATH1_DISPOSABLE_WORKER_ALLOCATION_ID" \
  --expected-runtime-release-digest "$CHARIOX_PATH1_EXPECTED_RUNTIME_RELEASE_DIGEST" \
  --project-id "$CHARIOX_PROJECT_ID" \
  --workspace-id "$CHARIOX_WORKSPACE_ID" \
  --worktree-id "$CHARIOX_WORKTREE_ID" \
  --target-platform linux-x86_64 \
  --validation-command 'sleep 5; command -v rustc' \
  --validation-command 'rustc --version' \
  --validation-command 'git rev-parse --show-toplevel'
```

For a direct home-kernel WebSocket, use `--kernel-url "$CHARIOX_KERNEL_URL"` and the existing local-auth environment. The defaults are provider `codex`, account profile `codex-1`, model `gpt-5.6-luna`, and effort `max`; pass explicit values when the selected existing account requires them. The prompt defaults to a read-only reply marker and may be overridden with `--prompt` only when the operator has reviewed its side effects.

The allocation GET uses the normal authenticated Cloud profile saved by the CLI; its session token is not a drill argument and is never written to the report. The expected digest is an independently supplied allocation release identity, not continuous attestation of the currently running binary.

The default report is written outside the repository under `~/.chariox/dev/project-environment-setup-acceptance/acceptance-report.json`; `--report` and `--artifact-root` may point to another external path. A passing report contains the exact Cloud account-scoped allocation ID, home/runtime identities, ready state, equal revisions, expiry, allocation release digest, platform, setup attempt/phase evidence, install-step count, validation command/passed/failed/output-byte counts, provider run ID, and launch/submit/completion counts. It contains no token or provider output.

## Current prerequisites and limits

The deployed 189 kernel is below the project-setup protocol 326 gate, so this drill must remain unrun until the selected home and Path1 worker are on the compatible signed release. The relay peer must also be 51 or newer. The selected Cloud deployment must expose the disposable-worker allocation GET and its persisted allocation release identity; a relay inventory or bootstrap response is not a substitute. This drill does not claim Path1 live acceptance until it has run against those real prerequisites.

This drill proves one real selected worker/project/provider path once run; it does not prove arbitrary workers, reconnect recovery, Cloud waiting-room UI behavior, full resource-cap enforcement, or lifecycle behavior outside the drill-owned session. The source contract is kernel-owned in `apps/kernel/src/runtime/state/project_environment_setup.rs`; the serialized request builders are in `packages/kernel-client/src/ipc-project-environment-setup-requests.ts`.
