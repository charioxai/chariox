# Candidate kernel startup and public protocol smoke

`candidate-kernel-startup-smoke-drill.mjs` is a bounded, local-only acceptance
drill for a packaged `chariox-kernel`. It is separate from provider, utility,
worker-VM, slice, and hosted-relay validation.

The caller supplies both the exact candidate binary and the expected local
daemon protocol. The drill first executes the candidate's
`--print-local-daemon-protocol-version` mode and fails before startup if the
reported value is not exact. It then starts that same binary with:

- fresh task-owned `CHARIOX_HOME`, `HOME`, XDG roots, history, log, and socket
  paths under an operating-system temporary directory;
- a fresh `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE` (mode `0600`), consumed by the
  candidate; and
- loopback-only kernel and runtime-MCP ports selected by the existing drill
  port helper.

The environment is deliberately allowlisted and has no relay, Cloud, event
delivery, provider-dev-stub, or process-environment local-auth variables. No
provider run, prompt, hosted relay, slice, or VM is started. The public
`LocalIpcClient` path performs:

1. authenticated `GetDaemonHealth` and process/relay assertions;
2. empty `ListSessions`;
3. `CreateSession`, `AttachToSession`, `GetSessionState`, and detach;
4. a fresh client `ListSessions`, `ResolveSession`, `GetSessionState`, attach,
   and detach; and
5. active-provider/active-prompt checks proving this smoke did not replay a
   prompt.

It also sends one request with a wrong local token and requires the public
client to reject it as `authentication_failed`. The drill counts successful
public requests and requires at least ten. It terminates only its own child
process and removes only its own temporary root, including on failure.

## Main-run prerequisite

The kernel candidate must already be built and executable, and the current
workspace must already contain the built TypeScript client at
`packages/kernel-client/dist/ipc.js` and `packages/kernel-client/dist/ipc-requests.js`.
This drill does not build either artifact.

From the exact OSS checkout, run:

```sh
node scripts/candidate-kernel-startup-smoke-drill.mjs \
  --binary /absolute/path/to/chariox-kernel \
  --expected-protocol 326 \
  --timeout-ms 60000
```

For this candidate, `--expected-protocol 325` is an intentional negative
preflight and must fail before the kernel starts. Any wrong or missing local
auth token is an intentional negative runtime check. A successful run proves
isolated candidate startup and public session protocol behavior only; it is not
real provider provisioning, toolchain installation, worker/VM readiness,
utility setup validation, relay compatibility, signed-release verification, or
live deployment acceptance.
