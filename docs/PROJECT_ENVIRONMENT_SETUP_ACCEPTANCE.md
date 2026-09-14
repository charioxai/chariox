# Project environment setup acceptance drill

`apps/cli/scripts/live-project-environment-setup-acceptance-drill.mjs` is the strict live Path1 acceptance drill. It is an operator drill, not a fixture or a readiness claim. This authoring change does not execute it.

The drill uses the public `LocalIpcClient` transport and the official kernel request builders. The home kernel must create a session with `kernel_ref` set to the selected worker kernel. That public `CreateSession` composition produces the remote-backed agent used by `StartProjectEnvironmentSetup`; the drill never injects a lease, worker receipt, provider result, or runtime binding.

## Acceptance sequence

The drill fails closed unless all of these are true:

- the endpoint is a WebSocket home-kernel or relay endpoint and the event transport is available;
- a read-only `GetProjectEnvironmentSetupStatus` probe reaches the selected home kernel, proving protocol 326 support. An `unknown variant` response is reported as an explicit minimum-version failure, while unrelated errors remain unverified;
- the exact worker machine is approved, online, not pending, and advertises the selected provider/account; the exact worker kernel belongs to that machine, accepts remote leases, and the created agent exposes relay peer protocol 51 or newer;
- the selected managed environment is `ready`/`running`, points at that exact machine and kernel, and supplies a non-empty trusted `runtimeReleaseDigest`. This is the reported worker runtime revision; no source revision is inferred from an alias or inventory label;
- the selected active Project exists, belongs to the selected workspace, has no existing environment definition, and the created session preserves the exact Project, workspace, and worktree;
- at least one real bounded validation command is supplied.

After preflight, the drill performs:

1. `StartProjectEnvironmentSetup` without a definition, with the requested validation commands. Kernel-owned utility execution derives the definition on the worker through the official provider path.
2. `GetProjectEnvironmentSetupStatus` polling. By default the drill requires an observed worker `preparing` or `validating` phase, then sends `CancelProjectEnvironmentSetup` and `RetryProjectEnvironmentSetup` and requires a new attempt before continuing. `--allow-terminal-before-cancel` is an explicit exception for a setup that became terminal before cancellation was applicable and records that gap in the report.
3. More `GetProjectEnvironmentSetupStatus` polling until authoritative `ready`. The status must identify the exact worker/platform and contain a non-empty, all-zero-exit validation result set. The Project is then re-read and must contain a utility-generated definition with non-zero setup/install steps and the requested validation commands.
4. Only after the observed `ready` status, one public `LaunchProviderRun`, one exact `SubmitPrompt`, and one matching `assistant_message_completed` event. The request log records the ordering and the provider run identity without recording prompt text, command text, provider output, or credentials.
5. Explicit event unsubscribe, attachment detach, and end-session cleanup for only the drill-created session. The persisted Project definition is intentionally retained as the user-owned setup result; the selected Project, worktree, managed environment, worker, and resource caps are never deleted or mutated by the drill.

## Invocation

Build the normal kernel-client package before a live run so `packages/kernel-client/dist/ipc.js` and `dist/ipc-requests.js` are present. The drill itself never builds them. Use existing deployed inputs; do not provision a machine or change credentials.

For a relay-connected home kernel, keep the token in the environment rather than the command line:

```sh
export CHARIOX_RELAY_TOKEN='provided-by-the-existing-runtime'
pnpm --filter @chariox/cli run project-environment:setup-acceptance -- \
  --relay-url "$CHARIOX_RELAY_URL" \
  --home-daemon-id "$CHARIOX_HOME_DAEMON_ID" \
  --worker-machine-id "$CHARIOX_PATH1_WORKER_MACHINE_ID" \
  --worker-kernel-id "$CHARIOX_PATH1_WORKER_KERNEL_ID" \
  --managed-environment-id "$CHARIOX_PATH1_MANAGED_ENVIRONMENT_ID" \
  --project-id "$CHARIOX_PROJECT_ID" \
  --workspace-id "$CHARIOX_WORKSPACE_ID" \
  --worktree-id "$CHARIOX_WORKTREE_ID" \
  --target-platform linux-x86_64 \
  --validation-command 'sleep 5; command -v rustc' \
  --validation-command 'rustc --version' \
  --validation-command 'git rev-parse --show-toplevel'
```

For a direct home-kernel WebSocket, use `--kernel-url "$CHARIOX_KERNEL_URL"` and the existing local-auth environment. The defaults are provider `codex`, account profile `codex-1`, model `gpt-5.5`, and effort `max`; pass explicit values when the selected existing account requires them. The prompt defaults to a read-only reply marker and may be overridden with `--prompt` only when the operator has reviewed its side effects.

The default report is written outside the repository under `~/.chariox/dev/project-environment-setup-acceptance/acceptance-report.json`; `--report` and `--artifact-root` may point to another external path. A passing report contains the exact worker machine/kernel, platform, managed runtime release digest, setup attempt/phase evidence, install-step count, validation command/passed/failed/output-byte counts, provider run ID, and launch/submit/completion counts. It contains no token or provider output.

## Current prerequisites and limits

The deployed 189 kernel is below the project-setup protocol 326 gate, so this drill must remain unrun until the selected home and Path1 worker are on the compatible signed release. The relay peer must also be 51 or newer. A public relay inventory does not independently carry the worker runtime revision, which is why a ready managed-environment record with matching runtime IDs and `runtimeReleaseDigest` is mandatory.

This drill proves one real selected worker/project/provider path once run; it does not prove arbitrary workers, reconnect recovery, Cloud waiting-room UI behavior, full resource-cap enforcement, or lifecycle behavior outside the drill-owned session. The source contract is kernel-owned in `apps/kernel/src/runtime/state/project_environment_setup.rs`; the serialized request builders are in `packages/kernel-client/src/ipc-project-environment-setup-requests.ts`.
