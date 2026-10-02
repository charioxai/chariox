# Phase 1 builder worker

The Mac kernel owns the sessions, prompts, permissions and history. The Hetzner
kernel hosts leased Codex processes. The relay carries encrypted kernel-to-kernel
packets over a Mac-origin SSH reverse tunnel. All listeners are loopback-only.

`apps/cli/scripts/cx-worker.mjs` drives both local and remote workers through the
home kernel. Set `CX_SESSION_DB` to an absolute state-file path outside source
checkouts and `CX_IPC_MODULE` to a built CLI `ipc.js` module if its default location
is unavailable. The live stack wrapper supplies both settings.

Run `new-remote` from a local git checkout. Its current checkout anchors the home
session; the supplied absolute path selects an existing checkout on the builder.
The worker kernel validates that path. The driver never resolves it on the Mac.

```sh
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs new-remote worker-name /w/worker-checkout
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs say worker-name /absolute/prompt.txt
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs wait worker-name 30
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs out worker-name
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs st
node /Users/miguel/.chariox/dev/apps-phase1/stack/cx.mjs drop worker-name
```

The default placement is `apps-phase1-builder-worker-g`. Override it with
`CX_REMOTE_KERNEL`. Agent defaults match the Phase 1 stack: provider `codex`,
model `gpt-6.1-sol`, high effort, profile `codex-1-6s6cnmim`, build mode and yolo
permissions. `CX_PROFILE` and `CX_MODEL` override the account and model.
`new` retains explicit local placement. Existing sessions keep their placement.

The selected Codex account replicates through the encrypted remote lease before
spawn. The target owns its replica and provider-native state. No auth files need
to be copied by an operator. `say`, `wait`, `out` and `st` use the home session's
normal protocol requests, including remote final text stored in history summaries.
A timed-out `wait` exits unsuccessfully.

GitHub access is a separate provisioning requirement. The existing SCM exporter
and materializer are currently wired to Cloud managed-context transfers.
`PrepareManagedEnvironmentGitCredentialEnrollment` requires a managed environment
and a Cloud-connected source kernel. A standalone leased worker cannot use that
request without those bindings. Public `git ls-remote` success does not prove
that `git push` or `gh` is authenticated. Do not repair this gap by copying tokens,
auth files or Keychain items.

The Phase 1 validation evidence, exact setup and stop commands live at
`/Users/miguel/.codex/evidence/chariox-apps-phase1/takeover/builder-worker/`.
The builder allocation ends on 2026-10-03 at 20:59 UTC. Its replicated account
profiles and kernel identities are durable credential state, not cleanup targets.
