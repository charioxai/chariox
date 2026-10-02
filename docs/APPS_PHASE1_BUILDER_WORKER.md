# Remote worker driver

The home kernel owns sessions, prompts, permissions and history. A remote kernel
hosts leased Codex processes. The relay carries encrypted kernel-to-kernel
packets. A worker behind SSH can reach the home relay through a reverse tunnel,
with all listeners bound to loopback.

`apps/cli/scripts/cx-worker.mjs` drives local and remote workers through the home
kernel. Set these operator-owned values before using it:

- `KERNEL_URL`: the home kernel's WebSocket URL.
- `CX_PROFILE`: the selected home Codex account profile, required for creation.
- `CX_REMOTE_KERNEL`: the registered worker kernel ID or alias, required for
  `new-remote`.
- `CX_SESSION_DB`: an absolute state-file path outside source checkouts.
- `CX_IPC_MODULE`: a built CLI `ipc.js` module, if the default location is absent.

Run `new-remote` from a local git checkout. Its current checkout anchors the home
session; the supplied absolute path selects an existing checkout on the worker.
The worker kernel validates that path. The driver never resolves it locally.

```sh
node apps/cli/scripts/cx-worker.mjs new-remote worker-name /absolute/worker-checkout
node apps/cli/scripts/cx-worker.mjs say worker-name /absolute/prompt.txt
node apps/cli/scripts/cx-worker.mjs wait worker-name 30
node apps/cli/scripts/cx-worker.mjs out worker-name
node apps/cli/scripts/cx-worker.mjs st
node apps/cli/scripts/cx-worker.mjs drop worker-name
```

The agent defaults to provider `codex`, model `gpt-6.1-sol`, high effort, build
mode and yolo permissions. `CX_MODEL` overrides the model. `new` retains explicit
local placement. Existing sessions keep their placement. `drop` destroys the
agent through the kernel's normal cleanup request before deleting its session.

The selected Codex account replicates through the encrypted remote lease before
spawn. The target owns its replica and provider-native state. No auth files need
to be copied by an operator. `say`, `wait`, `out` and `st` use the home session's
normal protocol requests, including remote final text stored in history summaries.
Failed or cancelled turns and timed-out waits exit unsuccessfully.

GitHub access is a separate provisioning requirement. The existing SCM exporter
and materializer are currently wired to Cloud managed-context transfers.
`PrepareManagedEnvironmentGitCredentialEnrollment` requires a managed environment
and a Cloud-connected source kernel. A standalone leased worker cannot use that
request without those bindings. Public `git ls-remote` success does not prove
that `git push` or `gh` is authenticated. Do not repair this gap by copying tokens,
auth files or Keychain items.

Replicated account profiles and kernel identities are durable credential state,
not cleanup targets. Keep operator-specific tunnel commands, service definitions,
expiry dates and validation receipts in external evidence and runtime directories.

Run the driver contract checks with `pnpm --filter @chariox/cli run test:cx-worker`.
