# Bounded encrypted-relay attachment recovery check

The existing `live-relay-runtime-drill.mjs` has a bounded
`--attachment-recovery` mode. It reuses that drill's relay startup, scoped HMAC
token fixture, isolated daemon environment, CLI module loader, target wait,
child process launcher, and cleanup path.

Run it from the repository root with staged, already-built executables:

```bash
node apps/cli/scripts/live-relay-runtime-drill.mjs \
  --attachment-recovery \
  --relay-binary /absolute/path/to/staged/chariox-relay \
  --kernel-binary /absolute/path/to/staged/chariox-kernel \
  --expected-protocol 326 \
  --timeout-ms 60000
```

The staged environment must already provide:

- executable `chariox-relay` and `chariox-kernel` files at the two supplied
  paths;
- the prebuilt production controller
  `apps/cli/dist/kernel-restart-recovery-controller.js`;
- the existing `apps/cli/node_modules` dependency tree and the prebuilt
  `packages/kernel-client/dist` package used by the CLI module loader.

This check does not build, install, deploy, start a service, or run Cargo. The
kernel protocol is checked by running the staged kernel's
`--print-local-daemon-protocol-version` probe before the relay/daemon children
are started. The mode starts exactly one drill relay and one drill kernel in
the existing isolated `~/.chariox/dev/browser-computer-use/relay-runtime/<stamp>`
root. Child handles are claimed by executable and Linux process start identity;
cleanup refuses to signal an unclaimed child and any cleanup error fails the
verdict.

The session is created once through the existing direct local readiness client,
matching the proven relay drill setup. Both acceptance clients are separate
`LocalIpcClient` instances connected to the relay URL with the existing scoped
relay token and target daemon alias. Every two-client attachment, state,
stale-rejection, recovery, and event-subscription operation uses the existing
encrypted relay transport; the outer local test relay is `ws://`, not a
TLS/browser test.

The acceptance ledger requires exactly 17 successful public requests:

1. daemon health and initial session list;
2. create one session and attach clients A and B;
3. read state with A, then authoritative state with both attachments;
4. pump terminal output through sibling B;
5. detach A and send a stale A pump, which must be rejected with
   `attachment_not_in_session`;
6. invoke the production `createKernelRestartRecoveryController` after that
   real rejection, using public relay requests for authoritative state and a
   replacement attach;
7. verify B remains attached, the replacement is distinct, the session/agent
   fingerprints are unchanged, and the session is not duplicated;
8. pump through B again, detach both attachments, and verify final state.

The outgoing request wrapper rejects `SubmitPrompt` and `SubmitPrompts`; no
provider run or prompt is launched. The production controller's event
subscription reset/resync path is exercised through the two relay clients.

Scope is limited to this local authenticated/encrypted relay command and event
transport, public session/agent identity, attachment membership, stale rejection,
replacement attach, sibling survival, and cleanup. It makes no claim about
browser rendering, page reload or browser persistence, live hosted relay,
provider prompts/threads/conversations, workspace/worktree persistence, or a
live home service restart.
