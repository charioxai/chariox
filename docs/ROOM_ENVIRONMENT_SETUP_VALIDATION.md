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

- Add the matching explicit setup control to Web using the same kernel request.
- Exercise create, bind, provision and display through a real kernel and worker
  without preconfiguring worker authority in a fixture.
- Verify saved-state restart of a legacy slice and live viewer/input/reconnect.
- Complete exact-head review and the remaining Browser/Computer plan gates.

The live missing-Environment-binding defect is not declared fixed by these
command tests. No existing live slice or agent was modified at this checkpoint.
