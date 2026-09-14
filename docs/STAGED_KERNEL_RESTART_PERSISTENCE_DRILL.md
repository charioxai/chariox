# Staged kernel restart-persistence acceptance drill

`staged-kernel-restart-persistence-drill.mjs` is a bounded, local-only
acceptance drill for one already-staged `chariox-kernel` binary. It reuses the
candidate startup smoke environment and the public `LocalIpcClient` requests;
it does not build, install, deploy, invoke Cargo, or launch a provider.

The caller supplies the exact executable and expected local-daemon protocol.
The drill rejects a non-regular/non-executable binary and runs the binary's
`--print-local-daemon-protocol-version` preflight before the kernel phase. A
protocol mismatch fails before the kernel child is started.

The kernel phase uses a fresh `mkdtemp` root containing task-owned `HOME`,
XDG roots, workspace, logs, history, socket paths, and external
`CHARIOX_HOME`. A private one-shot auth file is created for each process
generation with the same token and the same path. Loopback ports are selected
by the existing drill port helper. Relay, Cloud, event-delivery, and provider
environment variables are excluded.

The drill records a private ownership manifest for each directly spawned
kernel child. It records the PID, resolved binary, Linux process start
identity, generation, endpoint, `CHARIOX_HOME`, workspace, auth file, and
owned root. Before `SIGTERM` or bounded fallback `SIGKILL`, it verifies the
child handle, executable, argv[0], PID, and process start identity. It never
enumerates or signals installed kernels or unrelated processes. An occupied
loopback port is a hard refusal to proceed.

Public requests cover:

1. exact protocol preflight and authenticated health;
2. wrong-token rejection and an empty initial session list;
3. session creation, default-agent identity, attach, state, and detach;
4. supervised stop of the first child;
5. restart of the same exact binary with the same external `CHARIOX_HOME` and
   auth token;
6. health, list, resolve, and state requests proving the exact created
   session/agent IDs, references, durable fields, and no prompt replay after
   process restart; and
7. reattachment, one-agent/no-duplicate checks, detach, and final state.

The runnable path requires exactly 16 successful public requests, including
the post-restart session list taken after reattachment.

The expected session and agent fingerprints intentionally exclude transient
attachment IDs and activity timestamps that are changed by attaching a
client. They include the durable session/project/workspace/daemon identity,
session configuration, agent identity/reference, provider selection, grid
placement, state, and creation time. A lost session, same-ID replacement,
lost/replaced agent, or duplicate entity fails the verdict.

Both child shutdown and temporary-root removal are verified. A cleanup error
is included in the failed verdict; an unverified or still-running child keeps
the owned root in place for operator recovery. The drill does not claim
provider process/thread persistence, provider conversation persistence,
workspace/worktree persistence, browser/desktop persistence, relay recovery,
worker/VM persistence, or deployment acceptance.

## Main-run prerequisite

The staged kernel must already be built and executable, and the current
checkout must already contain the built TypeScript client at
`packages/kernel-client/dist/ipc.js` and
`packages/kernel-client/dist/ipc-requests.js`. This drill does not build
either artifact. The caller should run it in a capped private systemd unit;
this repository's focused helper tests do not launch a child kernel.

From this exact OSS checkout, run:

```sh
node scripts/staged-kernel-restart-persistence-drill.mjs \
  --binary /absolute/path/to/staged/chariox-kernel \
  --expected-protocol 326 \
  --timeout-ms 60000
```

`--expected-protocol 325` is an intentional negative preflight and must fail
before the kernel starts. The result is limited to the public local session
and daemon lifecycle checks listed above.
