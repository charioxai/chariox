The workspace drill exercises MP-08 / MP-10 / MP-11 through two disposable kernels, the built `chariox context copy` command, actual source TUI approval, and a hosted WSS relay. It compares linked Git HEAD, index, working files, binary/untracked content, executable state and a supporting directory. It also verifies refusal of an occupied divergent destination and a fresh return snapshot.

Run on the b3 validation host with these environment variables:

```sh
CHARIOX_WORKSPACE_KERNEL=/absolute/path/to/exact/chariox-kernel \
CHARIOX_WORKSPACE_KERNEL_HEAD=<full-kernel-commit> \
CHARIOX_WORKSPACE_TUI=/absolute/path/to/built/chariox \
CHARIOX_WORKSPACE_PROFILE=/absolute/path/to/product-linked/codex-profile \
CHARIOX_WORKSPACE_STATE=/absolute/path/to/unused/disposable-root \
CHARIOX_WORKSPACE_EVIDENCE=/absolute/path/to/evidence \
CHARIOX_WORKSPACE_ORIGIN=https://lane.val.51-255-87-178.sslip.io \
CHARIOX_WORKSPACE_RELAY=wss://lanerelay.val.51-255-87-178.sslip.io \
systemd-run --scope -p MemoryMax=4G node scripts/e2e-stack/movekernel/workspace-transfer.mjs
```

The origin must have normal validation device approval and support the current kernel enrollment contract. The harness links the existing profile through the product API to satisfy terminal admission; it submits no provider turn. Copy selects an empty kernel context and transfers only the selected Project development setup.

The divergence check observes only archive filenames in its own configured transfer store, then occupies the generated destination before import. It never calls private import helpers or constructs admission tickets. The target must refuse publication and preserve the existing inode, Git state and file tree. Runtime processes and disposable state are cleaned up; value-free JSON results and real TUI review captures remain in the evidence directory.

`durable-move.mjs` uses the same environment with a protocol634/237 kernel and
its real built TUI. It runs three short official Codex gpt-6-luna/low turns,
Moves forward, Returns, and restarts the home kernel after each source capture.
The same Room recalls the original marker and the intervening target turn;
completed shell output proves execution at each receipt's primary path. The
drill checks the supporting directory, latest target files, and preservation
of divergent original home work. It uses the public Move requests while the
route action UI remains a separate work item. Journal observation selects the
restart moment; it never changes a record, artifact, receipt, or provider state.
Cleanup retains only static native-probe stage metadata from kernel logs, then
removes the disposable state. Hosted Web captures prove existing Room visibility
at DPR 1 and 2; they do not prove the separately scoped Move route picker or
`via <target>` label.

MP-08 / MP-09 / MP-10 / MP-11: local protocol625 adds cancellation through
`MoveAgentToLocal: {session_id, agent_ref, abort_pending: true}`. This restores
the existing source route and returns `AgentMoveAborted`; it does not Return a
remote agent. Cancellation is refused once the move may hold a target lease.
Preflight checks source eligibility and native provider/interface/platform/CLI
compatibility before fencing. Return also checks its original home account.

MP-08 / MP-09 / MP-10 / MP-11: `composed-move.mjs` exercises PR4 with
local634/peer237. It starts with no target account, transfers the selected
profile through ordinary account materialization, runs the stored Project
setup on the target, and verifies matching home/target readiness and commit
receipts. One real source turn and one gpt-6-luna/low target turn prove the
same native session/account, a transferred granted skill, and a home-owned
MCP tool. The ignored setup output must be regenerated at the imported
primary path. The recipe also installs a tool under preparation HOME and adds
its directory through the definition PATH entries; the first target turn must
invoke it in a non-login shell, preserving the inherited Project PATH. This
proves the ordinary prepared provider environment survives Move and any
first-turn tool catalog refresh.
Use the environment above and the same 4 GiB scope. This kernel
acceptance leaves the route picker and its `via` label to PR5.
