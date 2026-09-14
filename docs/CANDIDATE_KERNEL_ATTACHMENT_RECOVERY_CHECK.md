# Candidate kernel-client attachment recovery check

`scripts/candidate-kernel-attachment-recovery-check.mjs` is a bounded,
local-only check for one already-staged `chariox-kernel` binary. It extends
the existing candidate startup and ownership runtime; it does not build,
install, deploy, invoke Cargo, or launch a provider.

The caller supplies the exact executable and expected local-daemon protocol.
The binary is checked before the protocol probe, and the probe must print the
exact expected protocol. The kernel is then started once with a fresh,
task-owned root containing an isolated external `CHARIOX_HOME`, workspace,
XDG roots, logs, history, socket paths, loopback ports, and a private one-shot
local-auth file. Relay, Cloud, event-registry, and provider environment
variables are excluded.

The one kernel child is recorded in the existing ownership manifest with its
resolved binary, PID, Linux process start identity, endpoint, state paths, and
phase. Only that verified child can receive the bounded TERM/KILL cleanup.
Occupied ports, a changed PID identity, an unverified child, a still-running
child, or temporary-root cleanup failure fails the verdict. The ownership root
is retained when safe cleanup cannot be proven.

The check uses two real `LocalIpcClient` instances against the same kernel:

1. Create one session and capture its exact durable session and default-agent
   identities.
2. Attach clients A and B and verify two distinct attachment IDs in one
   authoritative session state. Pump output through B while both attachments
   are present.
3. Detach A, send the stale A attachment through the public
   `PumpTerminalOutput` request, and require the exact
   `attachment_not_in_session` rejection. B is not detached or replaced.
4. Mark the real stale rejection as disconnected and invoke the production
   `createKernelRestartRecoveryController`. Its real public requests perform
   authoritative `GetSessionState` followed by `AttachToSession`; its local
   event-subscription reset/sync hooks are also bound to client A's local
   transport.
5. Verify the replacement attachment is distinct, B remains in the session,
   the session list has exactly one session, the exact agent is still the only
   agent, durable session/agent fields are unchanged, no prompt/provider run
   is active, and B can still pump output. Detach both attachments and verify
   the final authoritative state.

The runnable path requires exactly 17 successful control requests plus one
expected rejected stale-attachment request. It also requires the prebuilt
TypeScript artifacts used by the candidate drills:

* `packages/kernel-client/dist/ipc.js`
* `packages/kernel-client/dist/ipc-requests.js`
* `packages/kernel-client/dist/ipc-terminal-runtime-requests.js`
* `apps/cli/dist/kernel-restart-recovery-controller.js`

The drill's outgoing control-request boundary rejects `SubmitPrompt` and
`SubmitPrompts`; neither is sent. Focused tests use no child process. Run the
candidate check in a capped private systemd unit:

```sh
node scripts/candidate-kernel-attachment-recovery-check.mjs \
  --binary /absolute/path/to/staged/chariox-kernel \
  --expected-protocol 326 \
  --timeout-ms 60000
```

This check proves only local public session/agent identity, attachment
membership, stale-attachment rejection/replacement, and the CLI controller's
local recovery sequencing. It does not claim process-restart persistence,
provider process/thread or conversation persistence, workspace/worktree
persistence, browser rendering/page persistence, desktop persistence, live
relay recovery, worker/VM persistence, or deployment acceptance.
