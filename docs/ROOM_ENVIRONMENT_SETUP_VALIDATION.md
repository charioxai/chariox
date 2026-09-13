# Explicit Room Environment setup

This checkpoint exposes the existing kernel placement contract in local and
remote Chariox TUIs. It does not change a protocol shape or weaken admission.

From a terminal attached to the owning Room:

1. Run `/room bind SLICE` to reserve the selected headed slice as the default
   Environment. The kernel enforces Room ownership and rejects cross-Room or
   competing assignments. Repeating the same assignment is idempotent.
2. For a stopped slice, run `/slice start SLICE`. For an already-running unbound
   worker, use `/room save restart` to preserve its state and reinstall its Room
   binding through the provisioner. Busy-agent save rejection remains in force.
3. Use `/room start` for Environment runtime startup and `/room view` to open it.

Binding alone neither starts nor restarts a worker. A successful bind therefore
does not mean that display or controller admission is ready. Viewing does not
claim a slice or restart agents implicitly. There is no unbind/reassignment path
that could expose an existing browser profile to another Room.

## Focused validation

The new public slash-command regression failed before implementation because
`/room bind desktop` returned usage rather than sending the bind request. It now
passes. Four binding cases cover explicit mutation without restart, invalid or
detached input, kernel denial/malformed or wrong-Room responses, and the existing
protocol-282 minimum diagnostic.

48 Room command tests and 29 parser/submit tests passed using Node 25.1.0's
TypeScript transform and a task-local resolver for this worktree's source
packages. No generated packages, dependency installation, Rust build, live
provider turn, Docker container, or GitHub CI run was required. This was execution
validation, not a full TypeScript compilation.

## Open acceptance

- Deploy and click-test the matching Web setup control, implemented in Cloud
  draft PR 97, using the same kernel request.
- Verify saved-state restart of a legacy slice and live viewer/input/reconnect.
- Complete exact-head review and the remaining Browser/Computer plan gates.

The live missing-Environment-binding defect is not declared fixed by these
command tests. No existing live slice or agent was modified at this checkpoint.

## Real provisioning checkpoint, September 13

The public create → bind → provision → display-admission sequence passed on the
original Linux development machine with isolated kernel state. It used installed
kernel source `5e9898ce1a2dfcdde5222a00ce4d45f630b5b4c2` and a separately tagged
worker image built from that release's verified signed context and prebuilt
binaries. No worker Room binding was injected and no Rust rebuild was needed.

The kernel returned a Selkies endpoint with `access=tunnel` and
`stream_protocol=chariox-display-v1`. StopSlice, DeleteSlice,
DetachFromSession, and DeleteSession succeeded, the isolated kernel exited zero,
and no drill container, volume or listener remained. Existing agents and slices
were not touched. The worker had a 2 GiB memory and one-CPU limit.

Image: `sha256:5b395e45ba2849d8f34c8bbe04bc24d4647f145ccc3bcdd76f7ba9cdeafdb100`.
Signed context: `sha256:cf5fb8450ae5afd2f36c8a332e766d9910afe246da9ab9d4c4b56b146bd30ad7`.
Evidence is retained outside Git under
`/Users/miguel/.codex/evidence/browser-computer-use/room-setup/public-provision-signed-report.json`.

This closes only the fresh provisioning/admission check. It does not prove a
rendered frame, live input, reconnect, legacy saved-state restart, the Web button,
provider execution, Cloud VM Path 1, or active/idle soak acceptance.
